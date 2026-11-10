use super::*;
use std::{
    fs::File,
    os::windows::fs::OpenOptionsExt,
    sync::atomic::{AtomicUsize, Ordering},
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/annotations/synthetic.scrawl")
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let folder = std::env::temp_dir().join(format!(
            "sidenote-annotations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }
    fn bundle(&self, name: &str, json: &str) -> PathBuf {
        let folder = self.0.join(format!("{name}.scrawl"));
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("scrawl.json"), json).unwrap();
        folder
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn parses_fixture_and_preserves_mark_numbers_while_ordering_frames() {
    let capture = read_capture(&fixture()).unwrap();
    assert_eq!(capture.data.marks.len(), 3);
    assert_eq!(capture.word_count(), 12);
    assert_eq!(capture.app(), "Northwind Board");
    assert_eq!(
        capture
            .data
            .keyframes
            .iter()
            .map(|f| f.index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    for (i, mark) in capture.data.marks.iter().enumerate() {
        assert_eq!(
            capture.frame_for_mark(mark).unwrap().index,
            if i < 2 { 1 } else { 2 }
        );
    }
    assert!(
        capture
            .frame_path(&capture.data.keyframes[0])
            .unwrap()
            .ends_with("frames/frame-01.png")
    );
}

#[test]
fn tolerates_missing_optional_and_unknown_fields() {
    let temp = Temp::new();
    let folder = temp.bundle(
        "minimal",
        r#"{"id":"minimal","newField":42,"marks":[{}],"keyframes":[{"index":7,"unknown":true}]}"#,
    );
    let c = read_capture(&folder).unwrap();
    assert_eq!(c.app(), "Screen capture");
    assert_eq!(c.title(), "Untitled capture");
    assert_eq!(c.word_count(), 0);
    assert!(c.words_for_mark(&c.data.marks[0]).is_empty());
    assert!(c.frame_for_mark(&c.data.marks[0]).is_none());
    assert!(c.frame_path(&c.data.keyframes[0]).is_err());
    let nulls: Scrawl = serde_json::from_str(r#"{"target":{"app":null,"windowTitle":null},"marks":[{"frame":null,"boxNorm":null,"description":null}]}"#).unwrap();
    assert!(nulls.marks[0].frame.is_none());
    let pending: Scrawl =
        serde_json::from_str(r#"{"target":null,"transcript":null,"marks":null,"keyframes":null}"#)
            .unwrap();
    assert!(pending.marks.is_empty());
    assert!(pending.transcript.words.is_empty());
}

#[test]
fn word_window_includes_overlapping_words_at_both_three_second_boundaries() {
    assert_eq!(WORD_WINDOW_SECONDS, 3.0);
    let mut c = read_capture(&fixture()).unwrap();
    let mark = Mark {
        t0: 6.0,
        t1: 7.0,
        ..Default::default()
    };
    c.data.transcript.words = [
        (2.0, 2.99, "before"),
        (2.9, 3.0, "left"),
        (6.0, 6.5, "during"),
        (10.0, 10.2, "right"),
        (10.01, 11.0, "after"),
    ]
    .into_iter()
    .map(|(t0, t1, text)| Word {
        t0,
        t1,
        text: text.into(),
    })
    .collect();
    assert_eq!(
        c.words_for_mark(&mark)
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>(),
        ["left", "during", "right"]
    );
    let early = Mark {
        t0: 0.2,
        t1: 0.3,
        ..Default::default()
    };
    assert_eq!(c.words_for_mark(&early).len(), 2);
}

#[test]
fn frame_references_match_indices_or_files_without_guessing_nearest_time() {
    let c = read_capture(&fixture()).unwrap();
    for reference in [
        FrameRef::Index(2),
        FrameRef::File("frames\\frame-02.png".into()),
        FrameRef::File("frame-02.png".into()),
    ] {
        assert_eq!(
            c.frame_for_mark(&Mark {
                frame: Some(reference),
                ..Default::default()
            })
            .unwrap()
            .index,
            2
        );
    }
    assert!(
        c.frame_for_mark(&Mark {
            frame: Some(FrameRef::Index(999)),
            ..Default::default()
        })
        .is_none()
    );
}

#[test]
fn only_local_frame_pngs_can_reach_the_image_decoder() {
    let c = read_capture(&fixture()).unwrap();
    for file in [
        "../outside.png",
        "frames/../../outside.png",
        "C:/outside.png",
        "recording.mp4",
        "audio.wav",
        "frames/../scrawl.json",
        "https://example.invalid/a.png",
        "frames/frame-01.png:stream.png",
    ] {
        assert!(
            c.frame_path(&Keyframe {
                file: Some(file.into()),
                ..Default::default()
            })
            .is_err(),
            "{file}"
        );
    }
}

#[test]
fn oversized_bundle_is_skipped_before_manifest_or_video_is_read() {
    let temp = Temp::new();
    let folder = temp.bundle("large", "invalid JSON proves the size check runs first");
    let video = folder.join("recording.mp4");
    let mut f = File::create(&video).unwrap();
    // Sparse allocation makes this a metadata-only 2 GB test, not a 2 GB fixture.
    use std::os::windows::io::AsRawHandle;
    unsafe {
        let mut returned = 0;
        assert_ne!(
            windows_sys::Win32::System::IO::DeviceIoControl(
                f.as_raw_handle(),
                0x000900c4,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut()
            ),
            0
        );
    }
    f.set_len(BUNDLE_MAX_BYTES + 1).unwrap();
    use std::io::{Seek, Write};
    f.rewind().unwrap();
    f.write_all(b"private media").unwrap();
    drop(f);
    let _locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(video)
        .unwrap();
    let error = read_capture(&folder).unwrap_err();
    assert!(error.to_string().contains("exceeds 2 GB"));
    let scan = scan(&temp.0);
    assert!(scan.captures.is_empty());
    assert_eq!(scan.errors.len(), 1);
}

#[test]
fn scan_is_newest_first_and_does_not_open_audio_or_video() {
    let temp = Temp::new();
    temp.bundle("older", r#"{"createdAtMs":1}"#);
    let newest = temp.bundle("newest", r#"{"createdAtMs":2}"#);
    temp.bundle("bad", "not json");
    fs::write(newest.join("audio.wav"), "must not read").unwrap();
    fs::write(newest.join("recording.mp4"), "must not read").unwrap();
    let before = fs::read(newest.join("scrawl.json")).unwrap();
    let _audio = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(newest.join("audio.wav"))
        .unwrap();
    let _video = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(newest.join("recording.mp4"))
        .unwrap();
    let scan = scan(&temp.0);
    assert_eq!(scan.captures.len(), 2);
    assert_eq!(scan.captures[0].data.created_at_ms, 2);
    assert_eq!(scan.errors.len(), 1);
    assert_eq!(fs::read(newest.join("scrawl.json")).unwrap(), before);
    assert!(scan.captures[0].data.transcript.words.is_empty());
    assert!(super::scan(&temp.0.join("missing")).captures.is_empty());
}
