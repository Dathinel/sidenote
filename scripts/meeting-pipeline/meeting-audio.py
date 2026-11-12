"""CPU diarization/VAD, bounded speech cuts, and sequential warm Hex API passes."""
import argparse
import gc
import json
import math
import os
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request

import numpy as np
import soundfile as sf

RATE = 16000


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')


def load_audio(path, temp, ffmpeg):
    normalized = temp / (path.stem + '-16k.wav')
    if ffmpeg:
        subprocess.run([ffmpeg, '-nostdin', '-hide_banner', '-loglevel', 'error', '-y',
                        '-i', str(path), '-ac', '1', '-ar', str(RATE), '-c:a', 'pcm_s16le',
                        str(normalized)], check=True)
        audio, rate = sf.read(normalized, dtype='float32')
    else:
        from scipy.signal import resample_poly
        audio, rate = sf.read(path, dtype='float32', always_2d=True)
        audio = audio.mean(axis=1)
        if rate != RATE:
            divisor = math.gcd(rate, RATE)
            audio = resample_poly(audio, RATE // divisor, rate // divisor)
    return np.ascontiguousarray(audio, dtype='float32')


def merge_turns(turns, gap=1.5):
    merged = []
    for start, end, speaker in sorted(turns):
        if end <= start:
            continue
        if merged and speaker == merged[-1][2] and 0 <= start - merged[-1][1] < gap:
            merged[-1][1] = end
        else:
            merged.append([start, end, speaker])
    return merged


def split_turn(audio, start, end):
    # Search the last five seconds before the limit, avoiding tiny trailing pieces.
    while end - start > 24.4:
        left = start + 19.4
        right = min(start + 24.4, end - 1.0)
        candidates = np.arange(left, right + 0.001, 0.02)
        energies = [np.mean(np.square(audio[int((x-.04)*RATE):int((x+.04)*RATE)])) for x in candidates]
        cut = float(candidates[int(np.argmin(energies))])
        yield start, cut
        start = cut
    if end > start:
        yield start, end


def microphone_vad(audio, models):
    import sherpa_onnx as sh
    config = sh.VadModelConfig(sample_rate=RATE, num_threads=2, provider='cpu')
    config.silero_vad.model = str(models / 'silero_vad.onnx')
    config.silero_vad.min_silence_duration = .3
    config.silero_vad.min_speech_duration = .25
    config.silero_vad.max_speech_duration = 3600
    detector = sh.VoiceActivityDetector(config, buffer_size_in_seconds=60)
    turns = []
    def drain():
        while not detector.empty():
            segment = detector.front
            turns.append([segment.start / RATE, min(len(audio), segment.start + len(segment.samples)) / RATE, 'You'])
            detector.pop()
    for offset in range(0, len(audio), 512):
        block=audio[offset:offset+512]
        if len(block)<512: block=np.pad(block,(0,512-len(block)))
        detector.accept_waveform(block)
        drain()
    detector.flush()
    drain()
    return turns


def system_diarization(audio, models):
    import sherpa_onnx as sh
    config = sh.OfflineSpeakerDiarizationConfig(
        segmentation=sh.OfflineSpeakerSegmentationModelConfig(
            pyannote=sh.OfflineSpeakerSegmentationPyannoteModelConfig(
                model=str(models / 'sherpa-onnx-pyannote-segmentation-3-0/model.int8.onnx'),
                window_shift_ratio=.5), num_threads=2, provider='cpu'),
        embedding=sh.SpeakerEmbeddingExtractorConfig(
            model=str(models / '3dspeaker.onnx'), num_threads=2, provider='cpu'),
        # 0.5 split a 3-person call into 16 speakers; 0.8 gave 3 real ones plus short fragments.
        clustering=sh.FastClusteringConfig(num_clusters=-1, threshold=.8),
        min_duration_on=.25, min_duration_off=.3)
    if not config.validate():
        raise RuntimeError('Invalid diarization configuration; check models-extra ONNX files.')
    diarizer = sh.OfflineSpeakerDiarization(config)
    result = diarizer.process(audio).sort_by_start_time()
    speech = {}
    for r in result: speech[r.speaker] = speech.get(r.speaker, 0.) + r.end - r.start
    # Clusters with under 3 s of speech are fragments of a real speaker, not a new person.
    labels = {s: 'Them' for s, total in speech.items() if total < 3}
    turns = []
    numbered = 0
    for r in result:
        if r.speaker not in labels:
            numbered += 1
            labels[r.speaker] = f'Them-{numbered}'
        turns.append([max(0., r.start), min(len(audio)/RATE, r.end), labels[r.speaker]])
    return turns


def prepare(args):
    models=Path(args.models); meeting=Path(args.meeting); temp=Path(args.temp)
    chunks=[]; raw=[]; timings={}; started=time.perf_counter()
    for filename in ('system.wav','microphone.wav'):
        path=meeting/filename
        if not path.is_file(): continue
        tick=time.perf_counter()
        audio=load_audio(path,temp,args.ffmpeg)
        timings[filename+'_normalize_seconds']=time.perf_counter()-tick
        tick=time.perf_counter()
        turns=system_diarization(audio,models) if filename=='system.wav' else microphone_vad(audio,models)
        timings[filename+'_analysis_seconds']=time.perf_counter()-tick
        raw.extend(dict(track=filename,start=s,end=e,speaker=v) for s,e,v in turns)
        merged=merge_turns(turns)
        for i,(start,end,speaker) in enumerate(merged):
            for part,(s,e) in enumerate(split_turn(audio,start,end)):
                # Padding cannot include the adjacent speaker's turn on this track.
                previous=max((t[1] for t in merged[:i] if t[1]<=s),default=0.)
                following=min((t[0] for t in merged[i+1:] if t[0]>=e),default=len(audio)/RATE)
                a=max(0.,s-.3,previous); b=min(len(audio)/RATE,e+.3,following)
                if b<=a: continue
                identity=f'{path.stem}-{round(s*1000):09d}-{round(e*1000):09d}'
                wav=temp/(identity+'.wav')
                sf.write(wav,audio[round(a*RATE):round(b*RATE)],RATE,subtype='PCM_16')
                chunks.append(dict(Id=identity,Start=round(s,3),End=round(e,3),
                                   AudioStart=round(a,3),AudioEnd=round(b,3),Speaker=speaker,
                                   Track=filename,Path=str(wav),Telugu='',Hindi=''))
        del audio
        gc.collect()
    chunks.sort(key=lambda x:(x['Start'],x['Track'],x['Id']))
    timings['prepare_seconds']=time.perf_counter()-started
    save(temp/'chunks.json',chunks)
    save(temp/'audio-analysis.json',dict(timings=timings,turns=raw,segment_count=len(chunks),
                                       remote_speakers=len({r['speaker'] for r in raw if r['track']=='system.wav'})))
    print(json.dumps(dict(timings=timings,segments=len(chunks))),flush=True)


def post_audio(client, model, lang, audio):
    """One transcription call. Hex restarts when it auto-updates (a new port and token), so a refused
    connection re-reads the discovery file and waits for Hex to come back, up to 5 minutes."""
    for attempt in range(11):
        config=json.loads((Path(os.environ['APPDATA'])/'hex/local-api.json').read_text(encoding='utf-8'))
        port=int(config['port'])
        if not 1<=port<=65535: raise RuntimeError('Invalid Hex local API port')
        request=urllib.request.Request(f'http://127.0.0.1:{port}/transcriptions?model={model}&language={lang}',
            data=audio,headers={'Content-Type':'audio/wav','Authorization':'Bearer '+config['token']})
        try:
            with client.open(request,timeout=180) as response: return json.load(response)
        except urllib.error.HTTPError as error:
            # Never dump request headers, discovery file, or exception object.
            if error.code==401 and attempt<10: time.sleep(30); continue  # token rotated by a restart
            body_bytes = len(error.read(4096))
            raise RuntimeError(f'Hex local API HTTP {error.code}; captured body bytes={body_bytes}') from None
        except (urllib.error.URLError,ConnectionError):
            if attempt<10:
                print(f'Hex local API unreachable; retry {attempt+1}/10 in 30 s',flush=True); time.sleep(30); continue
            raise RuntimeError('Hex local API unavailable for 5 minutes; start Hex and retry. No settings were changed.') from None


def transcribe(args):
    temp=Path(args.temp); chunks=json.loads((temp/'chunks.json').read_text(encoding='utf-8'))
    # No proxy, even on machines with a global HTTP proxy configured.
    client=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    metrics={}; all_started=time.perf_counter()
    for model,lang,field in [(os.environ['HEX_TE_ASR_MODEL'],'te','Telugu'),('nemotron_3_5_asr','hi','Hindi')]:
        started=time.perf_counter(); segments=[]
        for index,chunk in enumerate(chunks):
            tick=time.perf_counter()
            data=post_audio(client,model,lang,Path(chunk['Path']).read_bytes())
            if not isinstance(data.get('transcript'),str): raise RuntimeError('Hex API response missing transcript')
            chunk[field]=data['transcript'].strip()
            elapsed=time.perf_counter()-tick; segments.append(elapsed)
            print(f'{lang} {index+1}/{len(chunks)} {chunk["Id"]}: {elapsed:.3f}s',flush=True)
        metrics[lang]={'seconds':time.perf_counter()-started,'segment_seconds':segments}
    metrics['asr_seconds']=time.perf_counter()-all_started
    save(temp/'chunks.json',chunks)
    save(temp/'asr-timings.json',metrics)


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('command',choices=['prepare','transcribe'])
    parser.add_argument('--temp',required=True)
    parser.add_argument('--meeting')
    parser.add_argument('--models')
    parser.add_argument('--ffmpeg',default='')
    args=parser.parse_args()
    prepare(args) if args.command=='prepare' else transcribe(args)
