"""CPU diarization/VAD, bounded speech cuts, and sequential warm Hex API passes."""
import argparse
import gc
import hashlib
import json
import math
import os
import struct
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request
import wave

import numpy as np
import soundfile as sf

RATE = 16000
LID_MODEL = 'd787948f922bd699e1cbcbeeeb16bf830514ec138bbbb89b516dccbc7283bbe5'


def read_json(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))


def routing_config(path):
    if not path:
        return None
    try:
        config = read_json(path)
        if config.get('labs', {}).get('language_detection') is True:
            return dict(labs={k: v for k, v in config['labs'].items()
                              if k in ('language_detection', 'language_detection_settings')},
                        pipeline={k: v for k, v in config.get('pipeline', {}).items()
                                  if k in ('language_mode', 'meeting_languages')})
    except (OSError, ValueError, AttributeError):
        pass
    return None


def pcm_identity(path):
    with wave.open(str(path), 'rb') as wav:
        if (wav.getnchannels(), wav.getsampwidth(), wav.getframerate(), wav.getcomptype()) != (1, 2, RATE, 'NONE'):
            raise ValueError('Routing requires mono PCM16 at 16 kHz')
        count = wav.getnframes()
        pcm = wav.readframes(count)
    if len(pcm) != count * 2 or count < 1600:
        raise ValueError('Truncated or short routing WAV')
    return pcm


def pcm_hash(pcm):
    # Match M5 pcm_sha256: signed PCM16 / 32768, encoded as little-endian f32.
    digest = hashlib.sha256()
    for (sample,) in struct.iter_unpack('<h', pcm):
        digest.update(struct.pack('<f', sample / 32768.0))
    return digest.hexdigest()


def chunk_identity(chunk):
    return {key: chunk[key] for key in
            ('Id', 'Start', 'End', 'AudioStart', 'AudioEnd', 'Speaker', 'Track', 'Path')}


def lid_request(chunks, languages):
    jobs = []
    identities = []
    if len({c['Id'] for c in chunks}) != len(chunks):
        raise ValueError('Duplicate chunk IDs')
    for index, chunk in enumerate(chunks):
        pcm = pcm_identity(chunk['Path'])
        frames = len(pcm) // 2
        # Equal slices cover padding too and avoid a tiny tail above the M5 limit.
        count = math.ceil(frames / (24 * RATE))
        windows = []
        hashes = []
        for part in range(count):
            a, b = frames * part // count, frames * (part + 1) // count
            windows.append(dict(id=f'{chunk["Id"]}-{part}', start=a / RATE,
                                end=b / RATE, audio_start=a / RATE,
                                track=chunk['Track'], speaker=chunk['Speaker'], turn=chunk['Id']))
            hashes.append(pcm_hash(pcm[a*2:b*2]))
        job = dict(id=f'chunk-{index}', wav=str(Path(chunk['Path']).resolve()), chunks=windows)
        jobs.append(job)
        identities.append(dict(chunk=chunk_identity(chunk), pcm=pcm_hash(pcm), hashes=hashes))
    return dict(meeting_languages=languages, jobs=jobs), identities


def checked_sidecars(request, identities, output, mode):
    rows = []
    for job, identity in zip(request['jobs'], identities):
        sidecar = read_json(output / job['id'] / 'language-detect.v1.json')
        if (sidecar.get('schema') != 'language-detect.v1' or sidecar.get('job') != job['id']
                or sidecar.get('source_wav') != job['wav']
                or sidecar.get('meeting_languages') != request['meeting_languages']
                or sidecar.get('model_sha256') != LID_MODEL or sidecar.get('language_mode') != mode
                or len(sidecar.get('chunks', [])) != len(job['chunks'])):
            raise ValueError('Mismatched language sidecar')
        for expected, actual, digest in zip(job['chunks'], sidecar['chunks'], identity['hashes']):
            for key in ('start', 'end', 'track', 'speaker', 'turn', 'audio_start'):
                if actual.get(key) != expected[key]:
                    raise ValueError('Mismatched language window')
            if actual.get('chunk') != expected['id']:
                raise ValueError('Mismatched language chunk')
            # Forced bypass has no detector PCM hash; adapter identity still binds the WAV.
            if actual.get('pcm_f32le_sha256') != digest and actual.get('status') != 'forced':
                raise ValueError('Mismatched language PCM')
        rows.append(dict(identity=identity, sidecar=sidecar))
    return rows


def observe(args):
    config = routing_config(args.routing_config)
    if config is None:
        return
    temp = Path(args.temp)
    # A fresh directory prevents a failed invocation from accepting older sidecars.
    output = temp / ('lid-' + str(time.time_ns()))
    output.mkdir()
    report = dict(schema='routing-observe.v1', rows=[], fallback_count=0, reason='detection_failed')
    try:
        chunks = read_json(temp / 'chunks.json')
        languages = config.get('pipeline', {}).get('meeting_languages', ['en', 'hi', 'te'])
        request, identities = lid_request(chunks, languages)
        request_path = output / 'request.json'
        snapshot = output / 'config.json'
        save(request_path, request)
        save(snapshot, config)
        mode = config.get('pipeline', {}).get('language_mode', 'auto')
        subprocess.run([args.lid_exe, '--language-detect', str(request_path),
                        '--lid-config', str(snapshot), '--lid-output', str(output)],
                       check=True, timeout=300, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        report['rows'] = checked_sidecars(request, identities, output, mode)
        report['reason'] = 'observed'
        report['config'] = config
        report['fallback_count'] = sum(any(w['status'] not in ('guess', 'forced')
                                         for w in r['sidecar']['chunks']) for r in report['rows'])
    except (OSError, ValueError, KeyError, TypeError, wave.Error, subprocess.SubprocessError):
        report['fallback_count'] = len(read_json(temp / 'chunks.json'))
    save(temp / 'routing-observe.json', report)
    print(json.dumps(dict(stage='routing-observe', reason=report['reason'],
                          fallback_count=report['fallback_count'])), flush=True)


def english_coverages(path, chunks):
    """Require a nonempty English reading over the entire speech interval on this track."""
    if not path:
        return set()
    intervals = {c['Id']: [] for c in chunks}
    for line in Path(path).read_text(encoding='utf-8-sig').splitlines():
        row = json.loads(line)
        track = {'microphone': 'microphone.wav', 'system': 'system.wav'}.get(row.get('source'))
        if isinstance(row.get('text'), str) and row['text'].strip():
            a, b = float(row['start_ms']) / 1000, float(row['end_ms']) / 1000
            if math.isfinite(a) and math.isfinite(b) and b > a:
                best, overlap = None, 0
                # Match the wrapper's single best-overlap assignment, including ties.
                for chunk in chunks:
                    if chunk['Track'] != track:
                        continue
                    size = min(max(b, a + .2), chunk['End'] + .5) - max(a, chunk['Start'] - .5)
                    if size > overlap:
                        best, overlap = chunk['Id'], size
                if best is not None:
                    intervals[best].append((a, b))
    covered = set()
    for chunk in chunks:
        cursor = chunk['Start']
        for a, b in sorted(intervals[chunk['Id']]):
            if a > cursor:
                break
            cursor = max(cursor, b)
        if cursor >= chunk['End']:
            covered.add(chunk['Id'])
    return covered


def english_coverage(path, chunk):
    return chunk['Id'] in english_coverages(path, [chunk])


def choose_route(sidecar, covered):
    """Return all models whenever the available evidence cannot justify a narrower set."""
    all_models = ['te', 'hi']
    observed = {lang for w in sidecar['chunks'] for lang in w.get('observed_languages', [])}
    observed.update(w['language'] for w in sidecar['chunks'] if w.get('language'))
    if len(observed) > 1:
        return all_models, 'mixed_evidence'
    languages = sidecar['meeting_languages']
    mode = sidecar['language_mode']
    if mode == 'off':
        return all_models, 'language_mode_off'
    if mode.startswith('forced:'):
        route = mode[7:]
        selected = ['en', 'hi'] if route == 'mixed' else route.split('+')
        if (not selected or any(s not in languages or s not in ('en', 'hi', 'te') for s in selected)
                or any(w.get('status') != 'forced' or w.get('forced_route') != route
                       for w in sidecar['chunks'])):
            return all_models, 'invalid_forced_route'
        reason = 'forced'
    else:
        if mode != 'auto' or sidecar.get('thresholds_calibrated') is not True:
            return all_models, 'uncalibrated'
        settings = sidecar.get('settings', {})
        threshold = settings.get('confidence_threshold', 0)
        margin = settings.get('margin_threshold', 0)
        if not (.9 <= threshold <= 1 and .2 <= margin <= 1):
            return all_models, 'invalid_thresholds'
        selected = set()
        for window in sidecar['chunks']:
            top = window.get('top_two', [])
            confidence = window.get('confidence')
            observed = window.get('observed_languages', [])
            if (window.get('status') != 'guess' or window.get('outside_meeting_languages') is not False
                    or not isinstance(confidence, (int, float)) or not threshold <= confidence <= 1
                    or len(top) != min(2, len(languages)) or not observed
                    or top[0].get('language') != window.get('language')
                    or top[0].get('confidence') != confidence
                    or not all(isinstance(s.get('confidence'), (int, float))
                               and 0 <= s['confidence'] <= 1 for s in top)
                    or confidence - (top[1]['confidence'] if len(top) > 1 else 0) < margin
                    or window.get('unrestricted_language') not in languages):
                return all_models, 'unchecked_or_uncertain'
            selected.update([window['language'], *observed])
        reason = 'checked'
    if not selected or any(s not in languages or s not in ('en', 'hi', 'te') for s in selected):
        return all_models, 'out_of_set'
    if 'en' in selected and not covered:
        return all_models, 'english_coverage_missing'
    return [s for s in all_models if s in selected], reason


def config_key(config):
    return hashlib.sha256(json.dumps(config, sort_keys=True).encode()).hexdigest()


def plan_routes(args):
    config = routing_config(args.routing_config)
    if config is None:
        return
    temp = Path(args.temp)
    chunks = read_json(temp / 'chunks.json')
    passes = {lang: dict(model=model, version='unreported') for model, lang, _ in asr_passes()}
    plan = dict(schema='routing-plan.v1', config_key=config_key(config), chunks=[], models=passes,
                detector=LID_MODEL, fallback_count=0)
    try:
        report = read_json(temp / 'routing-observe.json')
        if report.get('reason') != 'observed' or report.get('config') != config:
            raise ValueError('Missing current observation')
        by_id = {r['identity']['chunk']['Id']: r for r in report['rows']}
        if len(by_id) != len(chunks) or len(report['rows']) != len(chunks):
            raise ValueError('Incomplete observation')
    except (OSError, ValueError, KeyError, TypeError):
        by_id = {}
    try:
        coverage = english_coverages(getattr(args, 'english_reading', None), chunks)
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        coverage = set()
    for chunk in chunks:
        models, reason, identity = ['te', 'hi'], 'missing_or_mismatched', None
        try:
            row = by_id[chunk['Id']]
            identity = row['identity']
            request, current = lid_request([chunk], row['sidecar']['meeting_languages'])
            if identity != current[0]:
                raise ValueError('Stale PCM or chunk mapping')
            sidecar = row['sidecar']
            if (sidecar.get('schema') != 'language-detect.v1' or sidecar.get('model_sha256') != LID_MODEL
                    or sidecar.get('source_wav') != request['jobs'][0]['wav']
                    or sidecar.get('meeting_languages') != config.get('pipeline', {}).get('meeting_languages', ['en', 'hi', 'te'])
                    or sidecar.get('language_mode') != config.get('pipeline', {}).get('language_mode', 'auto')):
                raise ValueError('Mismatched sidecar header')
            expected = request['jobs'][0]['chunks']
            if len(sidecar['chunks']) != len(expected):
                raise ValueError('Incomplete windows')
            for actual, window, digest in zip(sidecar['chunks'], expected, identity['hashes']):
                if (any(actual.get(k) != v for k, v in window.items() if k != 'id')
                        or actual.get('chunk') != window['id']
                        or (actual.get('pcm_f32le_sha256') != digest and sidecar['language_mode'][:7] != 'forced:')):
                    raise ValueError('Mismatched window evidence')
            models, reason = choose_route(sidecar, chunk['Id'] in coverage)
        except (OSError, ValueError, KeyError, TypeError, AttributeError, wave.Error):
            pass
        fallback = reason not in ('checked', 'forced')
        plan['fallback_count'] += int(fallback)
        plan['chunks'].append(dict(id=chunk['Id'], identity=identity, models=models, reason=reason,
                                   fallback=fallback, route='+'.join(models) if models else 'en'))
    save(temp / 'routing-plan.json', plan)
    print(json.dumps(dict(stage='routing-plan', chunks=len(chunks), fallback_count=plan['fallback_count'])), flush=True)


def asr_passes():
    return [(os.environ['HEX_TE_ASR_MODEL'], 'te', 'Telugu'),
            ('nemotron_3_5_asr', 'hi', 'Hindi')]


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
    chunks=[]; raw=[]; timings={}; started=time.perf_counter(); duration_seconds=0.0
    for filename in ('system.wav','microphone.wav'):
        path=meeting/filename
        if not path.is_file(): continue
        tick=time.perf_counter()
        audio=load_audio(path,temp,args.ffmpeg)
        duration_seconds=max(duration_seconds,len(audio)/RATE)
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
                                       remote_speakers=len({r['speaker'] for r in raw if r['track']=='system.wav'}),
                                       duration_seconds=duration_seconds,asr_passes=len(asr_passes())))
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
    for model,lang,field in asr_passes():
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
    parser.add_argument('command',choices=['prepare','observe','plan','transcribe'])
    parser.add_argument('--temp',required=True)
    parser.add_argument('--meeting')
    parser.add_argument('--models')
    parser.add_argument('--ffmpeg',default='')
    parser.add_argument('--routing-config')
    parser.add_argument('--lid-exe')
    parser.add_argument('--english-reading')
    args=parser.parse_args()
    {'prepare': prepare, 'observe': observe, 'plan': plan_routes, 'transcribe': transcribe}[args.command](args)
