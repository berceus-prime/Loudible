//! Runs real FFmpeg over a hand-built, unencrypted M4A. Real AAX/AAXC decryption stays untested,
//! as in the Swift app, because no synthetic encrypted file exists.

use std::path::PathBuf;
use std::sync::Arc;

use loudible_core::*;
use loudible_ffmpeg::*;
use loudible_test_support::*;
use tokio_util::sync::CancellationToken;

const FRAMES: u32 = 400;
const SAMPLE_RATE: u32 = 44_100;
/// Not valid audio to play, but remuxing never decodes it.
const FRAME: [u8; 6] = [0x21, 0x10, 0x04, 0x60, 0x8c, 0x1c];

fn atom(name: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
    let payload = parts.concat();
    let mut out = (payload.len() as u32 + 8).to_be_bytes().to_vec();
    out.extend_from_slice(name);
    out.extend(payload);
    out
}

/// A full box with version 0 and the given flags.
fn full(name: &[u8; 4], flags: u32, parts: &[&[u8]]) -> Vec<u8> {
    let mut all: Vec<&[u8]> = Vec::new();
    let header = flags.to_be_bytes();
    all.push(&header);
    all.extend_from_slice(parts);
    atom(name, &all)
}

fn be32(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

const MATRIX: [u32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

/// An AAC-LC stereo M4A with `mdat` before `moov`.
fn synthetic_m4a() -> Vec<u8> {
    m4a_with_extra_moov_atoms(&[])
}

/// A valid 1x1 PNG: FFmpeg needs the dimensions, though it copies the bytes as they are.
const COVER: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90\x77\x53\xde\x00\x00\x00\x0c\x49\x44\x41\x54\x78\x9c\x63\x10\x50\x30\x00\x00\x00\xa4\x00\x61\x34\x66\x7d\x72\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

fn cover_atoms() -> Vec<u8> {
    let data = atom(b"data", &[&be32(&[14, 0]), COVER]);
    let ilst = atom(b"ilst", &[&atom(b"covr", &[&data])]);
    let handler = full(b"hdlr", 0, &[&[0; 4], b"mdirappl", &[0; 9]]);
    atom(b"udta", &[&full(b"meta", 0, &[&handler, &ilst])])
}

fn inherited_tag_atoms() -> Vec<u8> {
    let tags = [
        (b"\xa9ART", "Old Artist"),
        (b"aART", "Old Album Artist"),
        (b"\xa9wrt", "Old Composer"),
        (b"desc", "Old Description"),
        (b"cprt", "Old Copyright"),
        (b"\xa9day", "1999-01-01"),
    ];
    let entries: Vec<Vec<u8>> = tags
        .iter()
        .map(|(key, value)| atom(*key, &[&atom(b"data", &[&be32(&[1, 0]), value.as_bytes()])]))
        .collect();
    let ilst = atom(b"ilst", &[&entries.concat()]);
    let handler = full(b"hdlr", 0, &[&[0; 4], b"mdirappl", &[0; 9]]);
    atom(b"udta", &[&full(b"meta", 0, &[&handler, &ilst])])
}

fn m4a_with_extra_moov_atoms(extra: &[u8]) -> Vec<u8> {
    let ftyp = atom(b"ftyp", &[b"M4A \0\0\0\0isomM4A "]);
    let samples = FRAME.repeat(FRAMES as usize);
    let mdat = atom(b"mdat", &[&samples]);
    let offset = (ftyp.len() + 8) as u32;
    let duration = FRAMES * 1024;

    let esds = full(
        b"esds",
        0,
        &[
            &[3, 25, 0, 1, 0],
            &[4, 17, 0x40, 0x15, 0, 0, 0],
            &be32(&[128_000, 128_000]),
            &[5, 2, 0x12, 0x10],
            &[6, 1, 2],
        ],
    );
    let mp4a = atom(
        b"mp4a",
        &[
            &[0, 0, 0, 0, 0, 0, 0, 1],
            &[0; 8],
            &[0, 2, 0, 16, 0, 0, 0, 0],
            &(SAMPLE_RATE << 16).to_be_bytes(),
            &esds,
        ],
    );
    let stbl = atom(
        b"stbl",
        &[
            &full(b"stsd", 0, &[&be32(&[1]), &mp4a]),
            &full(b"stts", 0, &[&be32(&[1, FRAMES, 1024])]),
            &full(b"stsc", 0, &[&be32(&[1, 1, FRAMES, 1])]),
            &full(b"stsz", 0, &[&be32(&[FRAME.len() as u32, FRAMES])]),
            &full(b"stco", 0, &[&be32(&[1, offset])]),
        ],
    );
    let dinf = atom(
        b"dinf",
        &[&full(b"dref", 0, &[&be32(&[1]), &full(b"url ", 1, &[])])],
    );
    let minf = atom(b"minf", &[&full(b"smhd", 0, &[&[0; 4]]), &dinf, &stbl]);
    let mdia = atom(
        b"mdia",
        &[
            &full(
                b"mdhd",
                0,
                &[&be32(&[0, 0, SAMPLE_RATE, duration]), &[0x55, 0xc4, 0, 0]],
            ),
            &full(b"hdlr", 0, &[&[0; 4], b"soun", &[0; 12], b"Sound\0"]),
            &minf,
        ],
    );
    let track_seconds = duration / SAMPLE_RATE;
    let tkhd = full(
        b"tkhd",
        7,
        &[
            &be32(&[0, 0, 1, 0, track_seconds * 1000, 0, 0]),
            &[0, 0, 0, 0, 1, 0, 0, 0],
            &be32(&MATRIX),
            &be32(&[0, 0]),
        ],
    );
    let mvhd = full(
        b"mvhd",
        0,
        &[
            &be32(&[0, 0, 1000, track_seconds * 1000, 0x10000]),
            &[1, 0, 0, 0],
            &[0; 8],
            &be32(&MATRIX),
            &[0; 24],
            &be32(&[2]),
        ],
    );
    let trak = atom(b"trak", &[&tkhd, &mdia]);
    let moov = atom(b"moov", &[&mvhd, &trak, extra]);
    [ftyp, mdat, moov].concat()
}

struct Fixture {
    directory: TemporaryDirectory,
    converter: FfmpegConverter,
}

fn fixture() -> Fixture {
    let directory = TemporaryDirectory::default();
    std::fs::write(directory.join("in.aaxc"), synthetic_m4a()).unwrap();
    Fixture {
        directory,
        converter: FfmpegConverter::new().unwrap(),
    }
}

fn chapter(title: &str, start: i64, length: i64) -> Chapter {
    Chapter {
        title: title.into(),
        start_milliseconds: start,
        length_milliseconds: length,
    }
}

fn aaxc() -> Decryption {
    Decryption::Aaxc {
        key: vec![0x01; 16],
        iv: vec![0x02; 16],
    }
}

fn metadata(title: &str) -> BookMetadata {
    let book = Audiobook {
        subtitle: Some("A Subtitle".into()),
        authors: vec!["Ada Author".into(), "Bea Coauthor".into()],
        narrators: vec!["Nora Narrator".into()],
        publisher: Some("Example Audio".into()),
        release_date: Some("2021-03-04".into()),
        ..Audiobook::new("B000000001", Marketplace::US, title)
    };
    BookMetadata::from(&book)
}

impl Fixture {
    fn request(&self, input: &str, output: &str) -> ConversionRequest {
        ConversionRequest {
            input: self.directory.join(input),
            output: self.directory.join(output),
            decryption: aaxc(),
            chapters: vec![chapter("Opening", 0, 2000), chapter("Ending", 2000, 7000)],
            metadata: metadata("The Synthetic Novel"),
        }
    }

    async fn convert(&self, request: ConversionRequest) -> Result<(), ExporterError> {
        self.converter
            .convert(request, Arc::new(|_| {}), &CancellationToken::new())
            .await
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn position(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("{} missing", String::from_utf8_lossy(needle)))
}

fn read(path: PathBuf) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[tokio::test]
async fn remuxes_audio_with_chapters_and_metadata() {
    let f = fixture();
    let progress = Arc::new(Locked::new(Vec::new()));
    let values = progress.clone();
    f.converter
        .convert(
            f.request("in.aaxc", "out.m4b"),
            Arc::new(move |value| values.with(|all| all.push(value))),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    let output = read(f.directory.join("out.m4b"));
    assert!(
        contains(&output, &FRAME.repeat(FRAMES as usize)),
        "audio is copied unchanged"
    );
    // +faststart puts the index first, so players need not read the whole file.
    assert!(position(&output, b"moov") < position(&output, b"mdat"));
    for text in [
        "The Synthetic Novel",
        "Ada Author, Bea Coauthor",
        "Nora Narrator",
        "Audiobook",
        "ASIN: B000000001",
        "A Subtitle",
        "Example Audio",
        "2021-03-04",
        "Opening",
        "Ending",
    ] {
        assert!(contains(&output, text.as_bytes()), "{text} missing");
    }
    let values = progress.current();
    assert_eq!(values.last(), Some(&1.0));
    assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[tokio::test]
async fn absent_book_fields_clear_inherited_tags_in_the_m4b() {
    let f = fixture();
    std::fs::write(
        f.directory.join("tagged.aaxc"),
        m4a_with_extra_moov_atoms(&inherited_tag_atoms()),
    )
    .unwrap();
    let mut request = f.request("tagged.aaxc", "cleared.m4b");
    request.metadata = BookMetadata::from(&Audiobook::new("B1", Marketplace::US, "New Title"));
    f.convert(request).await.unwrap();
    let output = read(f.directory.join("cleared.m4b"));
    assert!(contains(&output, b"New Title"));
    for inherited in [
        "Old Artist",
        "Old Album Artist",
        "Old Composer",
        "Old Description",
        "Old Copyright",
        "1999-01-01",
    ] {
        assert!(
            !contains(&output, inherited.as_bytes()),
            "{inherited} survived the remux"
        );
    }
}

#[tokio::test]
async fn the_output_is_readable_by_ffmpeg_and_keeps_its_chapters() {
    let f = fixture();
    f.convert(f.request("in.aaxc", "first.m4b")).await.unwrap();
    let mut again = f.request("first.m4b", "second.m4b");
    again.chapters.clear();
    f.convert(again).await.unwrap();
    let output = read(f.directory.join("second.m4b"));
    assert!(contains(&output, b"Opening"));
    assert!(contains(&output, b"Ending"));
}

#[tokio::test]
async fn keeps_the_cover_art() {
    let f = fixture();
    std::fs::write(
        f.directory.join("cover.aaxc"),
        m4a_with_extra_moov_atoms(&cover_atoms()),
    )
    .unwrap();
    f.convert(f.request("cover.aaxc", "out.m4b")).await.unwrap();
    assert!(contains(&read(f.directory.join("out.m4b")), COVER));
}

#[tokio::test]
async fn accepts_aax_activation_bytes() {
    let f = fixture();
    let mut request = f.request("in.aaxc", "out.m4b");
    request.decryption = Decryption::Aax {
        activation_bytes: vec![0x1c, 0xeb, 0x00, 0xda],
    };
    f.convert(request).await.unwrap();
    assert!(f.directory.join("out.m4b").exists());
}

#[tokio::test]
async fn names_blank_chapters_and_drops_empty_ones() {
    let f = fixture();
    let mut request = f.request("in.aaxc", "out.m4b");
    request.chapters = vec![
        chapter("Skipped", 0, 0),
        chapter("", 0, 3000),
        chapter("Named", 3000, 6000),
    ];
    f.convert(request).await.unwrap();
    let output = read(f.directory.join("out.m4b"));
    assert!(contains(&output, b"Chapter 1"));
    assert!(contains(&output, b"Named"));
    assert!(!contains(&output, b"Skipped"));
}

#[tokio::test]
async fn text_stops_at_a_nul() {
    let f = fixture();
    let mut request = f.request("in.aaxc", "out.m4b");
    request.metadata = metadata("Before\0After");
    request.chapters = vec![chapter("Start\0Hidden", 0, 9000)];
    f.convert(request).await.unwrap();
    let output = read(f.directory.join("out.m4b"));
    assert!(contains(&output, b"Before"));
    assert!(contains(&output, b"Start"));
    assert!(!contains(&output, b"After"));
    assert!(!contains(&output, b"Hidden"));
}

#[tokio::test]
async fn handles_non_ascii_paths_and_titles() {
    let f = fixture();
    std::fs::copy(
        f.directory.join("in.aaxc"),
        f.directory.join("Ünï 日本語.aaxc"),
    )
    .unwrap();
    let mut request = f.request("Ünï 日本語.aaxc", "Çø 🎧.m4b");
    request.metadata = metadata("東京 Café");
    f.convert(request).await.unwrap();
    assert!(contains(
        &read(f.directory.join("Çø 🎧.m4b")),
        "東京 Café".as_bytes()
    ));
}

#[tokio::test]
async fn a_missing_input_is_described_and_leaves_nothing() {
    let f = fixture();
    let result = f.convert(f.request("absent.aaxc", "out.m4b")).await;
    assert!(
        matches!(&result, Err(ExporterError::Conversion(message)) if message.starts_with("Opening the download: ")),
        "{result:?}"
    );
    assert!(!f.directory.join("out.m4b").exists());
}

#[tokio::test]
async fn a_file_without_audio_is_rejected() {
    let f = fixture();
    std::fs::write(f.directory.join("junk.aaxc"), b"this is not audio").unwrap();
    let result = f.convert(f.request("junk.aaxc", "out.m4b")).await;
    assert!(
        matches!(result, Err(ExporterError::Conversion(_))),
        "{result:?}"
    );
    assert!(!f.directory.join("out.m4b").exists());
}

#[tokio::test]
async fn cancelling_first_produces_nothing() {
    let f = fixture();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = f
        .converter
        .convert(f.request("in.aaxc", "out.m4b"), Arc::new(|_| {}), &cancel)
        .await;
    assert_eq!(result, Err(ExporterError::Cancelled));
    assert!(!f.directory.join("out.m4b").exists());
}

#[tokio::test]
async fn cancelling_midway_stops_and_removes_the_output() {
    let f = fixture();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let result = f
        .converter
        .convert(
            f.request("in.aaxc", "out.m4b"),
            Arc::new(move |_| trigger.cancel()),
            &cancel,
        )
        .await;
    assert_eq!(result, Err(ExporterError::Cancelled));
    assert!(!f.directory.join("out.m4b").exists());
}

#[tokio::test]
async fn a_panicking_callback_fails_the_conversion_without_crossing_c() {
    let f = fixture();
    let result = f
        .converter
        .convert(
            f.request("in.aaxc", "out.m4b"),
            Arc::new(|_| panic!("synthetic panic")),
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(result, Err(ExporterError::Unexpected(_))),
        "{result:?}"
    );
    assert!(!f.directory.join("out.m4b").exists());
    // The worker survives.
    f.convert(f.request("in.aaxc", "after.m4b")).await.unwrap();
}

#[tokio::test]
async fn conversions_run_one_at_a_time_on_one_thread() {
    let f = fixture();
    let events = Arc::new(Locked::new(Vec::new()));
    let cancel = CancellationToken::new();
    let run = |name: &'static str, output: &'static str| {
        let events = events.clone();
        f.converter.convert(
            f.request("in.aaxc", output),
            Arc::new(move |_| {
                events.with(|all| all.push((name, std::thread::current().id())));
            }),
            &cancel,
        )
    };
    let (a, b, c) = tokio::join!(run("a", "a.m4b"), run("b", "b.m4b"), run("c", "c.m4b"));
    (a.and(b).and(c)).unwrap();

    let events = events.current();
    assert!(events.iter().all(|(_, thread)| *thread == events[0].1));
    assert_ne!(events[0].1, std::thread::current().id());
    let mut order: Vec<&str> = events.iter().map(|(name, _)| *name).collect();
    order.dedup();
    assert_eq!(order, ["a", "b", "c"]);
}

#[test]
fn metadata_tags_clear_what_the_book_lacks() {
    let tag = |key: &str, value: &str| (key.to_string(), value.to_string());
    let bare = BookMetadata::from(&Audiobook::new("B1", Marketplace::US, "T"));
    assert_eq!(
        metadata_tags(&bare),
        [
            tag("title", "T"),
            tag("album", "T"),
            tag("genre", "Audiobook"),
            tag("comment", "ASIN: B1"),
            tag("artist", ""),
            tag("album_artist", ""),
            tag("composer", ""),
            tag("description", ""),
            tag("copyright", ""),
            tag("date", ""),
        ]
    );
    assert_eq!(
        metadata_tags(&metadata("T"))[4..],
        [
            tag("artist", "Ada Author, Bea Coauthor"),
            tag("album_artist", "Ada Author, Bea Coauthor"),
            tag("composer", "Nora Narrator"),
            tag("description", "A Subtitle"),
            tag("copyright", "Example Audio"),
            tag("date", "2021-03-04"),
        ]
    );
}
