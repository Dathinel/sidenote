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


if __name__ == '__main__':
    unittest.main()
