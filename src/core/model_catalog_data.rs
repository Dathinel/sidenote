// Verified from pinned Hugging Face API metadata, 2026-11-09. See docs/models.md.
use super::model_catalog::{Model, ModelFile, Role, Runtime};
pub static MODELS: &[Model] = &[
    Model {
        id: "indicconformer-hi-ctc",
        name: "IndicConformer Hindi",
        languages: &["hi"],
        runtime: Runtime::SherpaOnnx,
        role: Role::Asr,
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/hi/model.int8.onnx",
                sha256: "915c71e04dd7e5378a4057fdebb252b3a587188e4e99db6d7ce0909ad5ad05fa",
                bytes: 197595593,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/tokens.txt",
                sha256: "ee60967630213f31951817ac8b402b92ec18cce80718a24a49b388e56672dfb2",
                bytes: 67605,
            },
        ],
        licence: "MIT",
        source: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/blob/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/README.md",
        // Advisory model memory from measured peak RSS, rounded up to decimal MB.
        minimum_ram_mb: Some(762),
        runtime_note: "CTC ONNX export. CPU speed is not evaluated in this lane.",
    },
    Model {
        id: "indicconformer-bn-ctc",
        name: "IndicConformer Bengali",
        languages: &["bn"],
        runtime: Runtime::SherpaOnnx,
        role: Role::Asr,
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/bn/model.int8.onnx",
                sha256: "e9120a534f69df065314be468bf15579f1b92a4cd8c07ad119b80b69244718a8",
                bytes: 197595578,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/tokens.txt",
                sha256: "ee60967630213f31951817ac8b402b92ec18cce80718a24a49b388e56672dfb2",
                bytes: 67605,
            },
        ],
        licence: "MIT",
        source: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/blob/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/README.md",
        // Minimum RAM is unmeasured for this export.
        minimum_ram_mb: None,
        runtime_note: "CTC ONNX export. CPU speed is not evaluated in this lane.",
    },
    Model {
        id: "indicconformer-ta-ctc",
        name: "IndicConformer Tamil",
        languages: &["ta"],
        runtime: Runtime::SherpaOnnx,
        role: Role::Asr,
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/ta/model.int8.onnx",
                sha256: "abb7b59d706b8d27ba3fb5e5e3db7671c9e1a09bf7bc6122de507c60030e65fb",
                bytes: 197595513,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/tokens.txt",
                sha256: "ee60967630213f31951817ac8b402b92ec18cce80718a24a49b388e56672dfb2",
                bytes: 67605,
            },
        ],
        licence: "MIT",
        source: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/blob/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/README.md",
        // Minimum RAM is unmeasured for this export.
        minimum_ram_mb: None,
        runtime_note: "CTC ONNX export. CPU speed is not evaluated in this lane.",
    },
    Model {
        id: "indicconformer-te-ctc",
        name: "IndicConformer Telugu",
        languages: &["te"],
        runtime: Runtime::SherpaOnnx,
        role: Role::Asr,
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/te/model.int8.onnx",
                sha256: "710f176e33080f1be84b4f38bb5f33be3f522dcf4f191bf4d5cc89f7a85924f3",
                bytes: 197595693,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/tokens.txt",
                sha256: "ee60967630213f31951817ac8b402b92ec18cce80718a24a49b388e56672dfb2",
                bytes: 67605,
            },
        ],
        licence: "MIT",
        source: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/blob/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/README.md",
        // Minimum RAM is unmeasured for this export.
        minimum_ram_mb: None,
        runtime_note: "CTC ONNX export. CPU speed is not evaluated in this lane.",
    },
    Model {
        id: "indicconformer-mr-ctc",
        name: "IndicConformer Marathi",
        languages: &["mr"],
        runtime: Runtime::SherpaOnnx,
        role: Role::Asr,
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/mr/model.int8.onnx",
                sha256: "1ea81e55c4b9b12624c9d02a5b9c1b6f7c871c78a55ff52d333f81cb5136eaf2",
                bytes: 197595593,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/tokens.txt",
                sha256: "ee60967630213f31951817ac8b402b92ec18cce80718a24a49b388e56672dfb2",
                bytes: 67605,
            },
        ],
        licence: "MIT",
        source: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/blob/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/README.md",
        // Minimum RAM is unmeasured for this export.
        minimum_ram_mb: None,
        runtime_note: "CTC ONNX export. CPU speed is not evaluated in this lane.",
    },
    Model {
        id: "whisper-hindi2hinglish-swift",
        name: "Whisper Hindi2Hinglish Swift",
        languages: &["hi", "en"],
        runtime: Runtime::SherpaOnnx,
        role: Role::Asr,
        files: &[
            ModelFile {
                name: "encoder.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/hi-hinglish-swift/encoder.int8.onnx",
                sha256: "817f10919051ba8c784935a54da22683fceefd45a8c3547e68369204fc36c5ff",
                bytes: 29122029,
            },
            ModelFile {
                name: "decoder.int8.onnx",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/hi-hinglish-swift/decoder.int8.onnx",
                sha256: "19e88530a38a00af1744bc2c9340e72d4687f017e19b37b1b6c69c2b5128fc12",
                bytes: 130781044,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/resolve/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/hi-hinglish-swift/tokens.txt",
                sha256: "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126",
                bytes: 816730,
            },
        ],
        licence: "Apache-2.0",
        source: "https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx/blob/9721eb71eea141fae0982cfcdb9dd2e3d4953c4a/README.md",
        // Advisory model memory from measured peak RSS, rounded up to decimal MB.
        minimum_ram_mb: Some(523),
        runtime_note: "Self-contained ONNX files. CPU speed is not evaluated in this lane.",
    },
    Model {
        id: "anuvani-lid-int8",
        name: "Anuvani language identification",
        languages: &[
            "as", "bn", "brx", "doi", "en", "gu", "hi", "kn", "ks", "kok", "mai", "ml", "mni",
            "mr", "ne", "or", "pa", "sa", "sat", "sd", "ta", "te", "ur",
        ],
        runtime: Runtime::SherpaOnnx,
        role: Role::LanguageId,
        files: &[
            ModelFile {
                name: "anuvani_lid.int8.onnx",
                url: "https://huggingface.co/CIIL/Anuvani-LID/resolve/3b9caa41c0a5ec5a09d38f3aecb2fa1dd6602531/mobile_inference/models/anuvani_lid.int8.onnx",
                sha256: "d787948f922bd699e1cbcbeeeb16bf830514ec138bbbb89b516dccbc7283bbe5",
                bytes: 168995871,
            },
            ModelFile {
                name: "labels.txt",
                url: "https://huggingface.co/CIIL/Anuvani-LID/resolve/3b9caa41c0a5ec5a09d38f3aecb2fa1dd6602531/labels.txt",
                sha256: "2ae318e9dd01d8f45327d8319c8c35feaed9e0edf6e76e0c606cbddfc170062b",
                bytes: 174,
            },
        ],
        licence: "Apache-2.0",
        source: "https://huggingface.co/CIIL/Anuvani-LID/blob/3b9caa41c0a5ec5a09d38f3aecb2fa1dd6602531/mobile_inference/docs/11-sherpa-onnx-patch.md",
        // Minimum RAM is unmeasured for this export.
        minimum_ram_mb: None,
        runtime_note: "Requires the publisher's patched sherpa-onnx 1.13.8. Opt-in; no runtime bundled here.",
    },
    Model {
        id: "nemotron_3_5_asr",
        name: "Nemotron 3.5 ASR in Hex",
        languages: &["hi", "en"],
        runtime: Runtime::Hex,
        role: Role::Asr,
        files: &[],
        licence: "OpenMDW-1.1",
        source: "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b/blob/ea30d66debe3740a08b573244286791d423d6b3e/README.md",
        // Minimum RAM is unmeasured for this export.
        minimum_ram_mb: None,
        runtime_note: "Use a copy already installed in Hex. Sidenote does not download or select it.",
    },
];
