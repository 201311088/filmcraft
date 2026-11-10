//! Adobe Premiere Pro projects (`.prproj`): import only.
//!
//! A `.prproj` is a gzip-compressed XML document (`<PremiereData Version="3">`). Its top-level
//! elements are objects with an `ObjectID` (a number) or an `ObjectUID` (a UUID); objects point at
//! one another with `ObjectRef` / `ObjectURef` attributes. Clean-room: the layout below was read off
//! projects saved by Premiere Pro 2026 (26.x) for this purpose, never from the application.
//!
//! What is read:
//!
//! - the bin tree (`RootProjectItem` → `ProjectItemContainer/Items`, `BinProjectItem` for bins);
//! - media (`ClipProjectItem` → `MasterClip` → `Clips` → `VideoClip`/`AudioClip` → `Source` →
//!   `VideoMediaSource`/`AudioMediaSource` → `Media`): the file path (`ActualMediaFilePath`, else
//!   `FilePath`), frame size, frame rate, duration, sample rate and start timecode (`Media/Start`);
//! - sequences (a clip whose source is a `VideoSequenceSource`/`AudioSequenceSource` → `Sequence`):
//!   frame size and rate from the `VideoTrackGroup`, sample rate from the `AudioTrackGroup`, video
//!   and audio tracks in `Index` order, and every clip on them (`*ClipTrackItem`: `Start` (absent =
//!   0) and `End` on the timeline; `SubClip` → `Clip` `InPoint`/`OutPoint` in the source; the
//!   `SubClip`'s `MasterClip` names the project item, so nested sequences come through too);
//! - links between clips (`Sequence/PersistentGroupContainer/LinkContainer/Links` → `Link`).
//!
//! Times need no conversion: Premiere counts 254 016 000 000 ticks per second, as FilmCraft does.
//! Frame rates are stored as ticks per frame (`8475667200` = 29.97 fps) and sample rates as ticks
//! per sample (`5292000` = 48 kHz).
//!
//! Not read yet (each is reported, never silently dropped): clip and track effects other than
//! the default Motion / Opacity / Volume / Channel Volume, transitions, markers, caption and data
//! tracks, track names and states, and project items other than media, sequences and bins.
//! Unverified against a saved project: nested bins (`BinProjectItem`), which follow the root bin's
//! layout.

use std::collections::HashMap;

use filmcraft_project::{BinId, ClipId, ItemId, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick};
use roxmltree::Node;

use crate::common::{Builder, MAX_IMPORTED_TRACKS, MediaSpec, empty_sequence, rate_from_f64, settings_for};
use crate::{Error, Format, ImportOptions, Imported, Report, Result};

/// The largest XML document a `.prproj` may inflate to. Real projects are a few MB to a few
/// hundred MB; a gzip bomb stops here instead of exhausting memory.
const MAX_XML_BYTES: usize = 1 << 30;
/// How deep bins may nest (and nested sequences may refer to each other) before the importer stops.
const MAX_DEPTH: usize = 64;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Whether `bytes` look like a Premiere project: gzip whose first kilobytes inflate to
/// `<PremiereData`, or the same XML uncompressed.
pub(crate) fn sniff(bytes: &[u8]) -> bool {
    let head = if bytes.starts_with(&GZIP_MAGIC) {
        match inflate(bytes, 4096) {
            Ok(xml) => xml,
            Err(Partial(xml)) => xml,
            Err(_) => return false,
        }
    } else {
        bytes.get(..bytes.len().min(4096)).unwrap_or_default().to_vec()
    };
    String::from_utf8_lossy(&head).contains("<PremiereData")
}

/// Why inflating stopped early.
enum InflateError {
    /// The output reached the limit: what was inflated up to it.
    Partial(Vec<u8>),
    /// Not gzip, or a damaged stream.
    Bad(String),
}
use InflateError::{Bad, Partial};

/// Inflate a gzip member (RFC 1952) into at most `limit` bytes.
fn inflate(bytes: &[u8], limit: usize) -> std::result::Result<Vec<u8>, InflateError> {
    if !bytes.starts_with(&GZIP_MAGIC) || bytes.get(2) != Some(&8) {
        return Err(Bad("not a gzip (deflate) stream".into()));
    }
    let flags = bytes.get(3).copied().unwrap_or(0);
    let mut pos = 10usize;
    // FEXTRA: a two-byte little-endian length, then that many bytes.
    if flags & 0x04 != 0 {
        let len = match (bytes.get(pos), bytes.get(pos + 1)) {
            (Some(&lo), Some(&hi)) => usize::from(u16::from_le_bytes([lo, hi])),
            _ => return Err(Bad("truncated gzip header".into())),
        };
        pos = pos.saturating_add(2).saturating_add(len);
    }
    // FNAME and FCOMMENT: zero-terminated strings.
    for flag in [0x08, 0x10] {
        if flags & flag != 0 {
            let rest = bytes.get(pos..).ok_or_else(|| Bad("truncated gzip header".into()))?;
            let nul = rest.iter().position(|&b| b == 0).ok_or_else(|| Bad("truncated gzip header".into()))?;
            pos = pos.saturating_add(nul).saturating_add(1);
        }
    }
    // FHCRC: a two-byte header checksum.
    if flags & 0x02 != 0 {
        pos = pos.saturating_add(2);
    }
    let body = bytes.get(pos..).ok_or_else(|| Bad("truncated gzip header".into()))?;
    miniz_oxide::inflate::decompress_to_vec_with_limit(body, limit).map_err(|e| match e.status {
        miniz_oxide::inflate::TINFLStatus::HasMoreOutput => Partial(e.output),
        status => Bad(format!("damaged gzip stream ({status:?})")),
    })
}

/// The XML text of a `.prproj` (gzip-compressed, or already uncompressed).
fn xml_text(bytes: &[u8]) -> Result<String> {
    let xml = if bytes.starts_with(&GZIP_MAGIC) {
        match inflate(bytes, MAX_XML_BYTES) {
            Ok(xml) => xml,
            Err(Partial(_)) => return Err(Error::parse(Format::Prproj, format!("the project inflates to more than {MAX_XML_BYTES} bytes"))),
            Err(Bad(message)) => return Err(Error::parse(Format::Prproj, message)),
        }
    } else {
        bytes.to_vec()
    };
    let xml = xml.strip_prefix(&[0xef, 0xbb, 0xbf]).map(<[u8]>::to_vec).unwrap_or(xml);
    String::from_utf8(xml).map_err(|_| Error::parse(Format::Prproj, "the project XML is not UTF-8"))
}

pub(crate) fn import(bytes: &[u8], opts: &ImportOptions, report: &mut Report) -> Result<Imported> {
    let text = xml_text(bytes)?;
    let doc = crate::xml::parse(&text).map_err(|e| Error::parse(Format::Prproj, e))?;
    let root = doc.root_element();
    if root.tag_name().name() != "PremiereData" {
        return Err(Error::parse(Format::Prproj, format!("expected <PremiereData>, found <{}>", root.tag_name().name())));
    }
    let mut objects = HashMap::new();
    for n in root.children().filter(Node::is_element) {
        if let Some(key) = n.attribute("ObjectID").or_else(|| n.attribute("ObjectUID")) {
            objects.entry(key).or_insert(n);
        }
    }
    let bin_root = root
        .children()
        .find(|n| n.has_tag_name("RootProjectItem"))
        .ok_or_else(|| Error::parse(Format::Prproj, "no RootProjectItem (the project's bin tree)"))?;
    let name = opts.name.clone().unwrap_or_else(|| "Imported Premiere Project".into());
    let mut imp = Imp { b: Builder::new(&name), objects, report, masters: HashMap::new(), sequences: Vec::new(), seen_sequences: HashMap::new() };
    imp.bin_items(bin_root, None, 0);
    let pending = std::mem::take(&mut imp.sequences);
    for (id, seq) in pending {
        imp.sequence(id, seq);
        imp.b.top.push(id);
    }
    if imp.b.p.items.is_empty() {
        return Err(Error::Empty);
    }
    Ok(imp.b.finish())
}

// ---------------------------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------------------------

/// The first element child of `n` named `name`.
fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}

/// The element at `path` below `n`, following the first child of each name.
fn at<'a, 'i>(n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
    path.iter().try_fold(n, |n, name| child(n, name))
}

/// The trimmed text of the element at `path` below `n`.
fn text<'a>(n: Node<'a, '_>, path: &[&str]) -> Option<&'a str> {
    at(n, path).and_then(|e| e.text()).map(str::trim).filter(|t| !t.is_empty())
}

/// The integer at `path` below `n`.
fn int(n: Node<'_, '_>, path: &[&str]) -> Option<i64> {
    text(n, path).and_then(|t| t.parse().ok())
}

/// The element children of `n` named `name`, in `Index` order when they carry one.
fn indexed<'a, 'i>(n: Option<Node<'a, 'i>>, name: &str) -> Vec<Node<'a, 'i>> {
    let mut v: Vec<Node> = n.into_iter().flat_map(|n| n.children()).filter(|c| c.has_tag_name(name)).collect();
    v.sort_by_key(|c| c.attribute("Index").and_then(|i| i.parse::<i64>().ok()).unwrap_or(i64::MAX));
    v
}

/// The key an `ObjectRef` / `ObjectURef` attribute points at.
fn ref_key<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    n.attribute("ObjectRef").or_else(|| n.attribute("ObjectURef"))
}

/// A rate stored as ticks per frame (or per sample), as frames (samples) per second.
fn per_second(ticks: i64) -> Option<f64> {
    (ticks > 0).then(|| TICKS_PER_SECOND as f64 / ticks as f64)
}

/// `FrameRect` text `"left,top,right,bottom"` as (width, height).
fn frame_size(rect: &str) -> Option<(u32, u32)> {
    let v: Vec<i64> = rect.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let [l, t, r, b] = v.as_slice() else { return None };
    let w = u32::try_from(r.checked_sub(*l)?).ok().filter(|&w| w > 0)?;
    let h = u32::try_from(b.checked_sub(*t)?).ok().filter(|&h| h > 0)?;
    Some((w, h))
}

// ---------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------

struct Imp<'a, 'i, 'r> {
    b: Builder,
    objects: HashMap<&'a str, Node<'a, 'i>>,
    report: &'r mut Report,
    /// `MasterClip` key → the project item it became (media or sequence).
    masters: HashMap<String, ItemId>,
    /// Sequences to fill once every project item exists (their clips refer to any of them).
    sequences: Vec<(ItemId, Node<'a, 'i>)>,
    /// `Sequence` object key → its project item (a video and an audio source share one sequence).
    seen_sequences: HashMap<&'a str, ItemId>,
}

impl<'a, 'i> Imp<'a, 'i, '_> {
    /// The object an element's `ObjectRef` / `ObjectURef` points at.
    fn deref(&self, n: Option<Node<'a, 'i>>) -> Option<Node<'a, 'i>> {
        n.and_then(ref_key).and_then(|k| self.objects.get(k)).copied()
    }

    /// The objects the `name` children of `list` point at.
    fn deref_all(&self, list: Option<Node<'a, 'i>>, name: &str) -> Vec<Node<'a, 'i>> {
        indexed(list, name).into_iter().filter_map(|c| self.deref(Some(c))).collect()
    }

    /// Import the items of a bin (the root bin, or a `BinProjectItem`).
    fn bin_items(&mut self, owner: Node<'a, 'i>, bin: Option<BinId>, depth: usize) {
        if depth > MAX_DEPTH {
            self.report.warn(format!("bins nested deeper than {MAX_DEPTH} levels were not imported"));
            return;
        }
        for item in self.deref_all(at(owner, &["ProjectItemContainer", "Items"]), "Item") {
            match item.tag_name().name() {
                "BinProjectItem" => {
                    let name = text(item, &["ProjectItem", "Name"]).unwrap_or("Bin");
                    let child_bin = self.b.bin(name, bin);
                    self.bin_items(item, Some(child_bin), depth + 1);
                }
                "ClipProjectItem" => self.clip_item(item, bin),
                other => self.report.warn(format!("Premiere project items of type {other} were not imported")),
            }
        }
    }

    /// A media file or a sequence in the Project panel.
    fn clip_item(&mut self, item: Node<'a, 'i>, bin: Option<BinId>) {
        let name = text(item, &["ProjectItem", "Name"]).unwrap_or("Clip").to_string();
        let Some(master) = self.deref(child(item, "MasterClip")) else {
            self.report.warn(format!("\"{name}\" has no master clip and was not imported"));
            return;
        };
        let Some(master_key) = master.attribute("ObjectUID").or_else(|| master.attribute("ObjectID")) else { return };
        let sources: Vec<Node> =
            self.deref_all(child(master, "Clips"), "Clip").into_iter().filter_map(|clip| self.deref(at(clip, &["Clip", "Source"]))).collect();

        // A sequence: its clips' sources are sequence sources.
        if let Some(seq) = sources.iter().find_map(|s| self.deref(at(*s, &["SequenceSource", "Sequence"]))) {
            let Some(seq_key) = seq.attribute("ObjectUID").or_else(|| seq.attribute("ObjectID")) else { return };
            let id = match self.seen_sequences.get(seq_key) {
                Some(&id) => id,
                None => {
                    let settings = self.sequence_settings(seq, &name);
                    let id = self.b.reserve_sequence(&name, settings, bin);
                    self.seen_sequences.insert(seq_key, id);
                    self.sequences.push((id, seq));
                    id
                }
            };
            self.masters.insert(master_key.to_string(), id);
            return;
        }

        // A media file: every source of the clip names the same `Media`.
        let Some(media) = sources.iter().find_map(|s| self.deref(at(*s, &["MediaSource", "Media"]))) else {
            let kinds: Vec<&str> = sources.iter().map(|s| s.tag_name().name()).collect();
            self.report.warn(format!("\"{name}\" ({}) is neither a media file nor a sequence and was not imported", kinds.join(", ")));
            return;
        };
        let Some(path) = text(media, &["ActualMediaFilePath"]).or_else(|| text(media, &["FilePath"])) else {
            self.report.warn(format!("\"{name}\" has no file path and was not imported"));
            return;
        };
        let video = self.deref(child(media, "VideoStream"));
        let audio = self.deref(child(media, "AudioStream"));
        let video_spec = video.and_then(|v| {
            let rate = rate_from_f64(per_second(int(v, &["FrameRate"])?)?);
            let (w, h) = text(v, &["FrameRect"]).and_then(frame_size)?;
            Some((w, h, rate))
        });
        let sample_rate =
            audio.and_then(|a| per_second(int(a, &["FrameRate"])?)).map(|r| r.round()).filter(|r| (1.0..=1_000_000.0).contains(r)).map(|r| r as u32);
        let duration = video
            .and_then(|v| int(v, &["Duration"]))
            .or_else(|| audio.and_then(|a| int(a, &["Duration"])))
            .or_else(|| sources.iter().find_map(|s| int(*s, &["OriginalDuration"])))
            .filter(|&d| d > 0)
            .map(Tick);
        // `Media/Start` is the first frame's timecode, as a time.
        let start_tc = video_spec.and_then(|(_, _, rate)| int(media, &["Start"]).filter(|&t| t >= 0).map(|t| rate.frame_at(Tick(t))));
        // The channel count is not in the project; linking the file reads the real one.
        let spec = MediaSpec { duration, video: video_spec, audio: sample_rate.map(|sr| (sr, 2)), start_tc, kind: None };
        let id = self.b.file_media(&format!("file:{path}"), &name, path, &spec, bin);
        self.masters.insert(master_key.to_string(), id);
    }

    /// Frame size, frame rate and sample rate of a sequence, from its track groups.
    fn sequence_settings(&mut self, seq: Node<'a, 'i>, name: &str) -> SequenceSettings {
        let groups = self.track_groups(seq);
        let video = groups.iter().find(|g| g.has_tag_name("VideoTrackGroup"));
        let audio = groups.iter().find(|g| g.has_tag_name("AudioTrackGroup"));
        let rate = video.and_then(|g| per_second(int(*g, &["TrackGroup", "FrameRate"])?)).map(rate_from_f64).unwrap_or(FrameRate::FPS_24);
        let (w, h) = video.and_then(|g| text(*g, &["FrameRect"]).and_then(frame_size)).unwrap_or((1920, 1080));
        let mut settings = settings_for(rate, w, h, false);
        if let Some(sr) = audio.and_then(|g| per_second(int(*g, &["TrackGroup", "FrameRate"])?)).map(|r| r.round()).filter(|r| (1.0..=1_000_000.0).contains(r))
        {
            settings.sample_rate = sr as u32;
        }
        if let Err(e) = settings.validate() {
            self.report.warn(format!("sequence \"{name}\" has settings FilmCraft cannot use ({e}); 1920x1080 at 24 fps was used"));
            settings = settings_for(FrameRate::FPS_24, 1920, 1080, false);
        }
        settings
    }

    fn track_groups(&self, seq: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
        indexed(child(seq, "TrackGroups"), "TrackGroup").into_iter().filter_map(|g| self.deref(child(g, "Second"))).collect()
    }

    /// Fill a reserved sequence with its tracks and clips.
    fn sequence(&mut self, id: ItemId, node: Node<'a, 'i>) {
        let name = self.b.p.item(id).map(|i| i.name.clone()).unwrap_or_default();
        let settings = self.b.p.sequence(id).map(|s| s.settings.clone()).unwrap_or_default();
        let mut seq = empty_sequence(settings);
        // Track item object key → the clip it became, for links.
        let mut clips: HashMap<&'a str, ClipId> = HashMap::new();
        for group in self.track_groups(node) {
            let kind = match group.tag_name().name() {
                "VideoTrackGroup" => TrackKind::Video,
                "AudioTrackGroup" => TrackKind::Audio,
                other => {
                    let used = self.deref_all(at(group, &["TrackGroup", "Tracks"]), "Track").iter().any(|t| !self.track_items(*t).is_empty());
                    if used {
                        self.report.warn(format!("sequence \"{name}\": {other} tracks (captions and data) were not imported"));
                    }
                    continue;
                }
            };
            for track_node in self.deref_all(at(group, &["TrackGroup", "Tracks"]), "Track") {
                let index = seq.tracks(kind).len();
                if index >= MAX_IMPORTED_TRACKS {
                    self.report.warn(format!("sequence \"{name}\": tracks beyond {MAX_IMPORTED_TRACKS} per kind were not imported"));
                    break;
                }
                let mut track = self.b.track(kind, index);
                for ti in self.track_items(track_node) {
                    if let Some(clip) = self.clip(ti, kind, &name) {
                        if let Some(key) = ti.attribute("ObjectID").or_else(|| ti.attribute("ObjectUID")) {
                            clips.insert(key, clip.id);
                        }
                        track.items.push(clip);
                    }
                }
                let transitions = indexed(at(track_node, &["ClipTrack", "TransitionItems", "TrackItems"]), "TrackItem").len();
                if transitions > 0 {
                    self.report.warn(format!("sequence \"{name}\": transitions were not imported ({transitions} on {})", track.name));
                }
                track.sort();
                seq.tracks_mut(kind).push(track);
            }
        }
        self.links(node, &mut seq, &clips);
        self.b.ensure_tracks(&mut seq, TrackKind::Video, 1);
        self.b.ensure_tracks(&mut seq, TrackKind::Audio, 1);
        self.b.put_sequence(id, seq);
    }

    /// The track item objects on a track, in order.
    fn track_items(&self, track: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
        self.deref_all(at(track, &["ClipTrack", "ClipItems", "TrackItems"]), "TrackItem")
    }

    /// One clip on a track.
    fn clip(&mut self, ti: Node<'a, 'i>, kind: TrackKind, seq_name: &str) -> Option<filmcraft_project::TrackItem> {
        let start = int(ti, &["ClipTrackItem", "TrackItem", "Start"]).unwrap_or(0);
        let end = int(ti, &["ClipTrackItem", "TrackItem", "End"])?;
        let duration = end.checked_sub(start).filter(|&d| d > 0)?;
        let sub = self.deref(at(ti, &["ClipTrackItem", "SubClip"]))?;
        let source_clip = self.deref(child(sub, "Clip"));
        let master_key = child(sub, "MasterClip").and_then(ref_key)?;
        let Some(&item) = self.masters.get(master_key) else {
            self.report.warn(format!("sequence \"{seq_name}\": a clip refers to a project item that was not imported"));
            return None;
        };
        let name = text(sub, &["Name"]).map(str::to_string).or_else(|| self.b.p.item(item).map(|i| i.name.clone())).unwrap_or_default();
        let source_in = source_clip.and_then(|c| int(c, &["Clip", "InPoint"])).unwrap_or(0).max(0);
        let source_out = source_clip.and_then(|c| int(c, &["Clip", "OutPoint"]));
        let mut clip = self.b.clip(item, kind, &name, Tick(start), Tick(duration), Tick(source_in));
        // A clip whose source range is not as long as its timeline range plays at another speed.
        if let Some(out) = source_out.filter(|&o| o > source_in) {
            let speed = (out - source_in) as f64 / duration as f64;
            if speed.is_finite() && (speed - 1.0).abs() > 1e-6 {
                clip.speed = speed;
                self.report.info(format!("sequence \"{seq_name}\": a clip's speed was derived from its source and timeline durations"));
            }
        }
        let effects = self
            .deref(at(ti, &["ClipTrackItem", "ComponentOwner", "Components"]))
            .map(|chain| indexed(at(chain, &["ComponentChain", "Components"]), "Component").len())
            .unwrap_or(0);
        if effects > 0 {
            self.report.warn(format!("sequence \"{seq_name}\": clip effects were not imported ({effects} on \"{name}\")"));
        }
        Some(clip)
    }

    /// Give the clips of each `Link` one link id, so they select and move together.
    fn links(&mut self, seq: Node<'a, 'i>, s: &mut filmcraft_project::Sequence, clips: &HashMap<&'a str, ClipId>) {
        for link in self.deref_all(at(seq, &["PersistentGroupContainer", "LinkContainer", "Links"]), "Link") {
            let members: Vec<ClipId> = indexed(at(link, &["TrackItemGroup", "TrackItems"]), "TrackItem")
                .into_iter()
                .filter_map(ref_key)
                .filter_map(|k| clips.get(k).copied())
                .collect();
            if members.len() < 2 {
                continue;
            }
            let id = self.b.link_id();
            for t in s.all_tracks_mut() {
                for c in t.items.iter_mut().filter(|c| members.contains(&c.id)) {
                    c.link = Some(id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rect() {
        assert_eq!(frame_size("0,0,3840,2160"), Some((3840, 2160)));
        assert_eq!(frame_size("10, 20, 1930, 1100"), Some((1920, 1080)));
        assert_eq!(frame_size("0,0,0,0"), None);
        assert_eq!(frame_size("0,0,-5,10"), None);
        assert_eq!(frame_size("x"), None);
    }

    #[test]
    fn rates_from_ticks() {
        assert_eq!(per_second(8_475_667_200).map(rate_from_f64), Some(FrameRate::new(30_000, 1001)));
        assert_eq!(per_second(5_292_000).map(f64::round), Some(48_000.0));
        assert_eq!(per_second(0), None);
        assert_eq!(per_second(-1), None);
    }

    #[test]
    fn damaged_gzip_is_an_error_not_a_panic() {
        assert!(xml_text(&[0x1f, 0x8b]).is_err());
        assert!(xml_text(&[0x1f, 0x8b, 8, 0x1c, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(xml_text(&[0x1f, 0x8b, 8, 0x04, 0, 0, 0, 0, 0, 0, 0xff, 0xff]).is_err());
        assert!(!sniff(&[0x1f, 0x8b, 8, 0x08]));
    }
}
