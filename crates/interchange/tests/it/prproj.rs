//! Premiere Pro project (`.prproj`) import, on small projects written here in the layout Premiere
//! saves (see the `prproj` module docs). The real project the layout was read from stays local:
//! it names the author's files.

use filmcraft_interchange::{Format, detect, import};
use filmcraft_project::{BinEntry, ItemKind, MediaRef, TrackKind};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};

const SEC: i64 = TICKS_PER_SECOND;
/// 29.97 fps as Premiere stores it: ticks per frame.
const FRAME_2997: i64 = 8_475_667_200;
/// 48 kHz as Premiere stores it: ticks per sample.
const SAMPLE_48K: i64 = 5_292_000;

/// A media clip (`ClipProjectItem` → `MasterClip` → `VideoClip`/`AudioClip` → sources → `Media`).
fn media(uid: &str, ids: u32, name: &str, path: &str, duration: i64) -> String {
    let (vc, ac, vs, as_, vst, ast, item) = (ids, ids + 1, ids + 2, ids + 3, ids + 4, ids + 5, ids + 6);
    format!(
        r#"
  <ClipProjectItem ObjectID="{item}"><ProjectItem><Name>{name}</Name></ProjectItem><MasterClip ObjectURef="{uid}"/></ClipProjectItem>
  <MasterClip ObjectUID="{uid}"><Clips><Clip Index="0" ObjectRef="{vc}"/><Clip Index="1" ObjectRef="{ac}"/></Clips><Name>{name}</Name></MasterClip>
  <VideoClip ObjectID="{vc}"><Clip><Source ObjectRef="{vs}"/></Clip></VideoClip>
  <AudioClip ObjectID="{ac}"><Clip><Source ObjectRef="{as_}"/></Clip></AudioClip>
  <VideoMediaSource ObjectID="{vs}"><MediaSource><Media ObjectURef="{uid}-media"/></MediaSource><OriginalDuration>{duration}</OriginalDuration></VideoMediaSource>
  <AudioMediaSource ObjectID="{as_}"><MediaSource><Media ObjectURef="{uid}-media"/></MediaSource></AudioMediaSource>
  <Media ObjectUID="{uid}-media"><AudioStream ObjectRef="{ast}"/><VideoStream ObjectRef="{vst}"/><Start>{start}</Start><FilePath>{path}</FilePath><Title>{name}</Title></Media>
  <VideoStream ObjectID="{vst}"><FrameRate>{FRAME_2997}</FrameRate><Duration>{duration}</Duration><FrameRect>0,0,3840,2160</FrameRect></VideoStream>
  <AudioStream ObjectID="{ast}"><FrameRate>{SAMPLE_48K}</FrameRate><Duration>{duration}</Duration></AudioStream>"#,
        // 01:00:00:00 at 29.97 (non-drop): 107892 frames.
        start = 107_892 * FRAME_2997,
    )
}

/// One project: a root bin holding `Footage` (two media files) and a sequence, which holds a linked
/// video + audio clip of `a.mov` (source 1–4 s at 0–3 s), `b.mov` at double speed (source 0–4 s
/// at 3–5 s, video only) and a transition on the video track.
fn project() -> String {
    let a = media("aaaaaaaa-0000-0000-0000-000000000001", 100, "a.mov", r"D:\Shoot\a.mov", 30 * SEC);
    let b = media("bbbbbbbb-0000-0000-0000-000000000002", 200, "b.mov", r"D:\Shoot\b.mov", 10 * SEC);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
  <Project ObjectRef="1"/>
  <RootProjectItem ObjectUID="root"><ProjectItem><Name>Root Bin</Name></ProjectItem>
    <ProjectItemContainer><Items><Item Index="0" ObjectRef="10"/><Item Index="1" ObjectRef="20"/></Items></ProjectItemContainer>
  </RootProjectItem>
  <BinProjectItem ObjectID="10"><ProjectItem><Name>Footage</Name></ProjectItem>
    <ProjectItemContainer><Items><Item Index="0" ObjectRef="106"/><Item Index="1" ObjectRef="206"/></Items></ProjectItemContainer>
  </BinProjectItem>
  {a}{b}
  <ClipProjectItem ObjectID="20"><ProjectItem><Name>Main Edit</Name></ProjectItem><MasterClip ObjectURef="seq-master"/></ClipProjectItem>
  <MasterClip ObjectUID="seq-master"><Clips><Clip Index="0" ObjectRef="21"/><Clip Index="1" ObjectRef="22"/></Clips></MasterClip>
  <VideoClip ObjectID="21"><Clip><Source ObjectRef="23"/></Clip></VideoClip>
  <AudioClip ObjectID="22"><Clip><Source ObjectRef="24"/></Clip></AudioClip>
  <VideoSequenceSource ObjectID="23"><SequenceSource><Sequence ObjectURef="seq"/></SequenceSource></VideoSequenceSource>
  <AudioSequenceSource ObjectID="24"><SequenceSource><Sequence ObjectURef="seq"/></SequenceSource></AudioSequenceSource>
  <Sequence ObjectUID="seq">
    <PersistentGroupContainer><LinkContainer><Links><Link Index="0" ObjectRef="40"/></Links></LinkContainer></PersistentGroupContainer>
    <TrackGroups>
      <TrackGroup Index="0"><First>video</First><Second ObjectRef="30"/></TrackGroup>
      <TrackGroup Index="1"><First>audio</First><Second ObjectRef="31"/></TrackGroup>
    </TrackGroups>
    <Name>Main Edit</Name>
  </Sequence>
  <VideoTrackGroup ObjectID="30"><TrackGroup><Tracks><Track Index="0" ObjectURef="vt1"/><Track Index="1" ObjectURef="vt2"/></Tracks><FrameRate>{FRAME_2997}</FrameRate></TrackGroup><FrameRect>0,0,3840,2160</FrameRect></VideoTrackGroup>
  <AudioTrackGroup ObjectID="31"><TrackGroup><Tracks><Track Index="0" ObjectURef="at1"/></Tracks><FrameRate>{SAMPLE_48K}</FrameRate></TrackGroup></AudioTrackGroup>
  <VideoClipTrack ObjectUID="vt1"><ClipTrack>
    <ClipItems><TrackItems><TrackItem Index="0" ObjectRef="50"/><TrackItem Index="1" ObjectRef="52"/></TrackItems></ClipItems>
    <TransitionItems><TrackItems><TrackItem Index="0" ObjectRef="60"/></TrackItems></TransitionItems>
  </ClipTrack></VideoClipTrack>
  <VideoClipTrack ObjectUID="vt2"><ClipTrack><ClipItems><TrackItems/></ClipItems></ClipTrack></VideoClipTrack>
  <AudioClipTrack ObjectUID="at1"><ClipTrack><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="51"/></TrackItems></ClipItems></ClipTrack></AudioClipTrack>
  <VideoClipTrackItem ObjectID="50"><ClipTrackItem><TrackItem><End>{a_end}</End></TrackItem><SubClip ObjectRef="70"/></ClipTrackItem></VideoClipTrackItem>
  <AudioClipTrackItem ObjectID="51"><ClipTrackItem><TrackItem><End>{a_end}</End></TrackItem><SubClip ObjectRef="71"/></ClipTrackItem></AudioClipTrackItem>
  <VideoClipTrackItem ObjectID="52"><ClipTrackItem><TrackItem><Start>{a_end}</Start><End>{b_end}</End></TrackItem><SubClip ObjectRef="72"/></ClipTrackItem></VideoClipTrackItem>
  <VideoTransitionTrackItem ObjectID="60"/>
  <SubClip ObjectID="70"><Clip ObjectRef="80"/><MasterClip ObjectURef="aaaaaaaa-0000-0000-0000-000000000001"/><Name>a.mov</Name></SubClip>
  <SubClip ObjectID="71"><Clip ObjectRef="81"/><MasterClip ObjectURef="aaaaaaaa-0000-0000-0000-000000000001"/><Name>a.mov</Name></SubClip>
  <SubClip ObjectID="72"><Clip ObjectRef="82"/><MasterClip ObjectURef="bbbbbbbb-0000-0000-0000-000000000002"/><Name>b fast</Name></SubClip>
  <VideoClip ObjectID="80"><Clip><InPoint>{sec}</InPoint><OutPoint>{four}</OutPoint></Clip></VideoClip>
  <AudioClip ObjectID="81"><Clip><InPoint>{sec}</InPoint><OutPoint>{four}</OutPoint></Clip></AudioClip>
  <VideoClip ObjectID="82"><Clip><OutPoint>{four}</OutPoint></Clip></VideoClip>
  <Link ObjectID="40"><TrackItemGroup><TrackItems><TrackItem Index="0" ObjectRef="50"/><TrackItem Index="1" ObjectRef="51"/></TrackItems></TrackItemGroup></Link>
</PremiereData>
"#,
        a_end = 3 * SEC,
        b_end = 5 * SEC,
        sec = SEC,
        four = 4 * SEC,
    )
}

/// `xml` as Premiere writes it: one gzip member (no file name, no extra fields).
fn gzip(xml: &str) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    out.extend(miniz_oxide::deflate::compress_to_vec(xml.as_bytes(), 6));
    // CRC-32 and size trailer; the importer does not check them.
    out.extend([0; 8]);
    out
}

#[test]
fn detects_gzip_and_plain_projects() {
    let xml = project();
    assert_eq!(detect(&gzip(&xml), Some("prproj")), Some(Format::Prproj));
    assert_eq!(detect(&gzip(&xml), None), Some(Format::Prproj), "by content, without the extension");
    assert_eq!(detect(xml.as_bytes(), Some("prproj")), Some(Format::Prproj));
    assert_eq!(Format::from_extension(".PRPROJ"), Some(Format::Prproj));
    assert!(!Format::ALL.contains(&Format::Prproj), "import only: no exporter lists it");
}

#[test]
fn imports_bins_media_sequence_clips_and_links() {
    let (imp, report) = import(&gzip(&project()), Format::Prproj, None).unwrap();
    let p = &imp.project;

    // Bins and media.
    let a = p.items.values().find(|i| i.name == "a.mov").unwrap();
    let footage = p
        .root
        .children
        .iter()
        .find_map(|e| match e {
            BinEntry::Bin(b) if b.name == "Footage" => Some(b),
            _ => None,
        })
        .expect("the Footage bin in the root bin");
    assert!(footage.children.contains(&BinEntry::Item(a.id)), "a.mov is in Footage");
    let ItemKind::Media(m) = &a.kind else { panic!("a.mov is media") };
    assert!(matches!(&m.media, MediaRef::File { path } if path == r"D:\Shoot\a.mov"));
    let v = m.info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.frame_rate), (3840, 2160, FrameRate::new(30_000, 1001)));
    assert_eq!(m.info.audio_streams.first().map(|s| s.sample_rate), Some(48_000));
    assert_eq!(m.info.duration, Tick(30 * SEC));
    assert_eq!(m.info.start_timecode, Some(107_892), "01:00:00:00 at 29.97");

    // The sequence.
    assert_eq!(imp.sequences.len(), 1);
    let seq_item = p.item(imp.sequences[0]).unwrap();
    assert_eq!(seq_item.name, "Main Edit");
    assert!(p.root.children.contains(&BinEntry::Item(seq_item.id)), "the sequence is in the root bin");
    let seq = p.sequence(seq_item.id).unwrap();
    assert_eq!((seq.settings.width, seq.settings.height, seq.settings.frame_rate), (3840, 2160, FrameRate::new(30_000, 1001)));
    assert_eq!(seq.settings.sample_rate, 48_000);
    assert_eq!(seq.tracks(TrackKind::Video).len(), 2, "empty tracks are kept");
    assert_eq!(seq.tracks(TrackKind::Audio).len(), 1);

    // Clips: timeline range, source range, speed.
    let v1 = &seq.tracks(TrackKind::Video)[0].items;
    assert_eq!(v1.len(), 2);
    let (va, vb) = (&v1[0], &v1[1]);
    assert_eq!((va.item, va.start, va.duration, va.source_in, va.speed), (a.id, Tick(0), Tick(3 * SEC), Tick(SEC), 1.0));
    assert_eq!((vb.start, vb.duration, vb.source_in, vb.speed), (Tick(3 * SEC), Tick(2 * SEC), Tick(0), 2.0));
    assert_eq!(vb.name, "b fast");
    let aa = &seq.tracks(TrackKind::Audio)[0].items[0];
    assert_eq!((aa.item, aa.start, aa.duration, aa.source_in), (a.id, Tick(0), Tick(3 * SEC), Tick(SEC)));

    // The Link groups the video and audio of a.mov, and nothing else.
    assert!(va.link.is_some());
    assert_eq!(va.link, aa.link);
    assert_eq!(vb.link, None);

    // What was not imported is reported.
    assert!(report.mentions("transitions were not imported"), "{report}");
    assert!(report.mentions("speed was derived"), "{report}");
}

#[test]
fn plain_xml_imports_like_gzip() {
    let (gz, _) = import(&gzip(&project()), Format::Prproj, None).unwrap();
    let (plain, _) = import(project().as_bytes(), Format::Prproj, None).unwrap();
    assert_eq!(gz.project.items.len(), plain.project.items.len());
    assert_eq!(gz.sequences.len(), plain.sequences.len());
}

#[test]
fn a_clip_of_a_missing_item_is_reported_not_imported() {
    let xml = project()
        .replace(r#"<MasterClip ObjectURef="bbbbbbbb-0000-0000-0000-000000000002"/><Name>b fast"#, r#"<MasterClip ObjectURef="nowhere"/><Name>b fast"#);
    let (imp, report) = import(xml.as_bytes(), Format::Prproj, None).unwrap();
    let seq = imp.project.sequence(imp.sequences[0]).unwrap();
    assert_eq!(seq.tracks(TrackKind::Video)[0].items.len(), 1);
    assert!(report.mentions("not imported"), "{report}");
}

#[test]
fn documents_that_are_not_projects_are_errors() {
    assert!(import(b"<?xml version=\"1.0\"?><xmeml/>", Format::Prproj, None).is_err());
    assert!(import(b"<PremiereData Version=\"3\"/>", Format::Prproj, None).is_err(), "no bin tree");
    assert!(import(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1, 2, 3], Format::Prproj, None).is_err(), "damaged deflate");
}

/// Self-referencing bins stop at the depth limit instead of recursing forever.
#[test]
fn a_bin_that_contains_itself_is_bounded() {
    let xml = project().replace(r#"<Item Index="1" ObjectRef="206"/>"#, r#"<Item Index="1" ObjectRef="206"/><Item Index="2" ObjectRef="10"/>"#);
    let (_, report) = import(xml.as_bytes(), Format::Prproj, None).unwrap();
    assert!(report.mentions("nested deeper"), "{report}");
}

/// Truncated and bit-flipped projects (compressed and plain) return a result or an error, never a panic.
#[test]
fn damaged_projects_never_panic() {
    let xml = project();
    let gz = gzip(&xml);
    for doc in [xml.as_bytes().to_vec(), gz] {
        let step = (doc.len() / 97).max(1);
        for cut in (0..doc.len()).step_by(step) {
            let mut flipped = doc.clone();
            if let Some(b) = flipped.get_mut(cut) {
                *b ^= 0x5a;
            }
            for bytes in [doc.get(..cut).unwrap_or_default().to_vec(), flipped] {
                let r = std::panic::catch_unwind(|| {
                    let _ = detect(&bytes, Some("prproj"));
                    let _ = import(&bytes, Format::Prproj, None);
                });
                assert!(r.is_ok(), "panicked at byte {cut}");
            }
        }
    }
}
