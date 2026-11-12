"""Synthetic routing proofs. Run on an approved box, never on the laptop."""
import copy
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import wave

# Routing and fake-engine tests deliberately need no inference dependencies.
sys.modules.setdefault('numpy', types.ModuleType('numpy'))
sys.modules.setdefault('soundfile', types.ModuleType('soundfile'))
spec = importlib.util.spec_from_file_location('meeting_audio', Path(__file__).with_name('meeting-audio.py'))
audio = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audio)


class RoutingTests(unittest.TestCase):
    def setUp(self):
        model_env = patch.dict(audio.os.environ, HEX_TE_ASR_MODEL='synthetic-te-v1')
        model_env.start()
        self.addCleanup(model_env.stop)
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.wav = self.root / 'synthetic.wav'
        self.pcm = struct.pack('<hhhh', -32768, -1, 0, 32767) * (16000 * 25 // 4)
        with wave.open(str(self.wav), 'wb') as wav:
            wav.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
            wav.writeframes(self.pcm)
        self.chunk = dict(Id='system-1000-25400', Start=1., End=25.4, AudioStart=.7,
                          AudioEnd=25.7, Speaker='speaker-a', Track='system.wav', Path=str(self.wav),
                          Telugu='', Hindi='')

    def sidecars(self):
        request, identities = audio.lid_request([self.chunk], ['en', 'hi', 'te'])
        job = request['jobs'][0]
        rows = [dict(w, chunk=w['id'], pcm_f32le_sha256=digest, status='guess')
                for w, digest in zip(job['chunks'], identities[0]['hashes'])]
        sidecar = dict(schema='language-detect.v1', job=job['id'], source_wav=job['wav'],
                       meeting_languages=request['meeting_languages'], model_sha256=audio.LID_MODEL,
                       language_mode='auto', chunks=rows)
        target = self.root / job['id']
        target.mkdir(exist_ok=True)
        audio.save(target / 'language-detect.v1.json', sidecar)
        return request, identities, sidecar

    def test_full_pcm_and_padding_mapping(self):
        request, identities, _ = self.sidecars()
        windows = request['jobs'][0]['chunks']
        self.assertEqual(windows[0]['audio_start'], 0)
        self.assertEqual(windows[-1]['end'], 25)
        self.assertEqual(windows[0]['end'], windows[1]['start'])
        self.assertEqual(identities[0]['chunk']['AudioStart'], .7)
        self.assertEqual(identities[0]['pcm'], audio.pcm_hash(self.pcm))
        self.assertEqual(audio.pcm_hash(struct.pack('<h', -32768)),
                         audio.hashlib.sha256(struct.pack('<f', -1.)).hexdigest())
        self.assertEqual(len(audio.checked_sidecars(request, identities, self.root, 'auto')), 1)

    def test_mismatches_reject_sidecar(self):
        request, identities, sidecar = self.sidecars()
        for key, value in [('chunk', 'wrong'), ('audio_start', .3), ('track', 'wrong'),
                           ('pcm_f32le_sha256', 'stale')]:
            changed = copy.deepcopy(sidecar)
            changed['chunks'][0][key] = value
            audio.save(self.root / 'chunk-0/language-detect.v1.json', changed)
            with self.assertRaises(ValueError):
                audio.checked_sidecars(request, identities, self.root, 'auto')

    def test_disabled_does_no_work(self):
        args = types.SimpleNamespace(routing_config=None)
        with patch.object(audio.subprocess, 'run') as run:
            audio.observe(args)
        run.assert_not_called()
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ['synthetic.wav'])

    def test_failed_detector_is_advisory(self):
        config = self.root / 'config.json'
        audio.save(config, {'labs': {'language_detection': True}})
        audio.save(self.root / 'chunks.json', [self.chunk])
        with patch.object(audio.subprocess, 'run', side_effect=OSError):
            audio.observe(types.SimpleNamespace(temp=str(self.root), routing_config=str(config), lid_exe='fake'))
        report = audio.read_json(self.root / 'routing-observe.json')
        self.assertEqual(report['rows'], [])
        self.assertEqual(report['fallback_count'], 1)

    def evidence(self, languages=('en', 'hi', 'te'), winners=('te',)):
        return dict(meeting_languages=list(languages), language_mode='auto', thresholds_calibrated=True,
                    settings=dict(confidence_threshold=.9, margin_threshold=.2), chunks=[dict(
                        status='guess', outside_meeting_languages=False, language=winners[0],
                        confidence=.98, top_two=[dict(language=winners[0], confidence=.98),
                                                 dict(language='hi', confidence=.01)],
                        unrestricted_language=winners[0], observed_languages=list(winners))])

    def test_routes_and_switches(self):
        for winners, expected in [(('te',), ['te']), (('hi',), ['hi']), (('en',), []),
                                  (('en', 'te'), ['te', 'hi']), (('en', 'hi'), ['te', 'hi']),
                                  (('te', 'hi'), ['te', 'hi'])]:
            with self.subTest(winners=winners):
                reason = 'mixed_evidence' if len(winners) > 1 else 'checked'
                self.assertEqual(audio.choose_route(self.evidence(winners=winners), True), (expected, reason))
                if len(winners) > 1:
                    for calibrated in (False, True):
                        evidence = self.evidence(winners=winners)
                        evidence['thresholds_calibrated'] = calibrated
                        for covered in (False, True):
                            self.assertEqual(audio.choose_route(evidence, covered), (['te', 'hi'], 'mixed_evidence'))
        evidence = self.evidence()
        evidence['chunks'].append(self.evidence(winners=('hi',))['chunks'][0])
        self.assertEqual(audio.choose_route(evidence, True)[0], ['te', 'hi'])

    def test_uncertainty_always_runs_all(self):
        changes = [('status', 'silence_unchecked'), ('status', 'short_without_same_turn_predecessor'),
                   ('status', 'detection_failed_unchecked'), ('status', 'uncertain'),
                   ('confidence', None), ('confidence', .89), ('outside_meeting_languages', True),
                   ('unrestricted_language', 'ta'), ('observed_languages', ['ta']),
                   ('top_two', []), ('observed_languages', [])]
        for key, value in changes:
            evidence = self.evidence()
            evidence['chunks'][0][key] = value
            self.assertEqual(audio.choose_route(evidence, True)[0], ['te', 'hi'], (key, value))
        evidence = self.evidence()
        evidence['thresholds_calibrated'] = False
        self.assertEqual(audio.choose_route(evidence, True), (['te', 'hi'], 'uncalibrated'))
        self.assertEqual(audio.choose_route(self.evidence(winners=('en',)), False)[0], ['te', 'hi'])

    def test_forced_routes_and_declarations(self):
        for forced, expected in [('te', ['te']), ('hi', ['hi']), ('en', []),
                                 ('mixed', ['hi']), ('te+hi', ['te', 'hi']), ('ta', ['te', 'hi'])]:
            evidence = self.evidence()
            evidence['language_mode'] = 'forced:' + forced
            evidence['chunks'] = [dict(status='forced', forced_route=forced)]
            self.assertEqual(audio.choose_route(evidence, True)[0], expected)

    def test_coverage_requires_full_track_interval(self):
        path = self.root / 'synthetic.ndjson'
        rows = [dict(source='system', start_ms=1000, end_ms=10000, text='synthetic'),
                dict(source='system', start_ms=10000, end_ms=25400, text='synthetic')]
        path.write_text('\n'.join(json.dumps(r) for r in rows))
        self.assertTrue(audio.english_coverage(path, self.chunk))
        rows[1]['start_ms'] = 10001
        path.write_text('\n'.join(json.dumps(r) for r in rows))
        self.assertFalse(audio.english_coverage(path, self.chunk))
        rows[1]['start_ms'] = 10000
        rows[1]['source'] = 'microphone'
        path.write_text('\n'.join(json.dumps(r) for r in rows))
        self.assertFalse(audio.english_coverage(path, self.chunk))

    def test_english_line_cannot_cover_two_chunks(self):
        reading = self.root / 'synthetic.ndjson'
        reading.write_text(json.dumps(dict(source='system', start_ms=0, end_ms=20000, text='synthetic')))
        chunks = [dict(self.chunk, Id='first', Start=0, End=10),
                  dict(self.chunk, Id='second', Start=10, End=20)]
        self.assertEqual(audio.english_coverages(reading, chunks), {'first'})

    def prepare_plan(self):
        request, identities, sidecar = self.sidecars()
        for window in sidecar['chunks']:
            window.update(self.evidence()['chunks'][0])
        sidecar.update(thresholds_calibrated=True, settings=self.evidence()['settings'])
        config_path = self.root / 'config.json'
        audio.save(config_path, dict(labs=dict(language_detection=True)))
        config = audio.routing_config(config_path)
        audio.save(self.root / 'chunks.json', [self.chunk])
        audio.save(self.root / 'routing-observe.json', dict(reason='observed', config=config,
                   rows=[dict(identity=identities[0], sidecar=sidecar)]))
        args = types.SimpleNamespace(temp=str(self.root), routing_config=str(config_path), english_reading=None)
        audio.plan_routes(args)
        return args

    def test_plan_binds_pcm_and_complete_evidence(self):
        args = self.prepare_plan()
        self.assertEqual(audio.read_json(self.root / 'routing-plan.json')['chunks'][0]['models'], ['te'])
        report = audio.read_json(self.root / 'routing-observe.json')
        report['rows'][0]['sidecar']['chunks'].pop()
        audio.save(self.root / 'routing-observe.json', report)
        audio.plan_routes(args)
        self.assertEqual(audio.read_json(self.root / 'routing-plan.json')['fallback_count'], 1)
        self.prepare_plan()
        with wave.open(str(self.wav), 'wb') as wav:
            wav.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
            wav.writeframes(b'\0\0' * (16000 * 25))
        audio.plan_routes(args)
        self.assertEqual(audio.read_json(self.root / 'routing-plan.json')['chunks'][0]['models'], ['te', 'hi'])


if __name__ == '__main__':
    unittest.main()
