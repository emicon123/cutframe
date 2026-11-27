//! Audio pace per voice (clip-speed-audio.md §5.3): [`AudioPace`] (speed, reverse, pitch) and
//! [`TimeStage`], the per-voice time-stretch or resample stage on FFmpeg's `libavfilter`.
//!
//! The stage is a filter graph `abuffer -> ... -> aformat -> abuffersink` on interleaved stereo
//! `f32` at one sample rate:
//!
//! - pitch preserved: `atempo` (WSOLA), chained so each instance's factor is within 0.5..=2
//!   (`n = ceil(|log2 s|)` instances of `s^(1/n)`; at most two up to 4x);
//! - pitch not preserved: `asetrate = rate x percent / 100` (an exact integer for every rate
//!   divisible by 100: 44.1, 48, 88.2, 96, 176.4, 192 kHz) then `aresample = rate`: plain
//!   resampling, faster and higher like tape.
//!
//! It is built on the feeder or export thread (it allocates), never on the real-time callback.
//!
//! **Alignment.** `atempo` plays its output early: output frame `y` carries source frame
//! `(y + lead) x speed`, with a `lead` of 7 to 27 ms that depends on the factor and the rate but
//! hardly on the signal (WSOLA aligns each fragment by correlation, within a window). The lead
//! of each graph kind is measured once per process on a synthetic signal ([`stage_lead`]). After
//! a reset at source time `x` the caller feeds from [`TimeStage::preroll_frames`] before `x`;
//! the stage drops the output in front of `x` (a whole window for `atempo`, which also hides
//! its fade-in), so content lands where the clip mapping says within about 4.5 ms, and the
//! output never drifts (`tests/audio_pace.rs`). `signalsmith-stretch` is the documented upgrade
//! path behind the same type (design §5.4).

use std::collections::HashMap;
use std::f64::consts::PI;
use std::ffi::{CStr, CString};
use std::sync::{Mutex, OnceLock};

use app_core::{Clip, MAX_SOUNDING_SPEED, Speed, Time};
use rsmpeg::avfilter::{AVFilter, AVFilterGraph, AVFilterInOut};
use rsmpeg::avutil::{AVChannelLayout, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;

use super::sample_offset;
use crate::media::{MediaError, MediaErrorKind};

/// How one voice plays its source: speed, direction and pitch handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioPace {
    pub speed: Speed,
    /// Reads the source downwards (reversed clip).
    pub reverse: bool,
    /// With a speed other than 1x: time-stretch (`atempo`) instead of resampling.
    pub preserve_pitch: bool,
}

impl AudioPace {
    /// 1x forward: no stage, today's path.
    pub const NORMAL: AudioPace = AudioPace {
        speed: Speed::NORMAL,
        reverse: false,
        preserve_pitch: true,
    };

    /// The pace of `clip`'s voice (clips without audio props keep the pitch).
    pub fn of(clip: &Clip) -> Self {
        Self {
            speed: clip.speed(),
            reverse: clip.reversed(),
            preserve_pitch: clip.audio.as_ref().is_none_or(|a| a.preserve_pitch),
        }
        .normalized()
    }

    /// `preserve_pitch` means nothing at 1x: normalized to true, so a pitch toggle on a 1x clip
    /// does not count as a pace change.
    pub fn normalized(self) -> Self {
        Self {
            preserve_pitch: self.preserve_pitch || self.speed == Speed::NORMAL,
            ..self
        }
    }

    /// Whether the voice sounds at all: voices faster than [`MAX_SOUNDING_SPEED`] (4x) are
    /// silent in preview and export, and not even read.
    pub fn sounds(self) -> bool {
        self.speed.percent() <= MAX_SOUNDING_SPEED.percent()
    }

    /// Whether the voice needs a [`TimeStage`] (any speed but 1x).
    pub fn stretches(self) -> bool {
        self.speed != Speed::NORMAL
    }

    /// Source time covered by `span` of output (timeline) time: `floor(span x p / 100)`.
    pub fn source_span(self, span: Time) -> Time {
        self.speed.source_floor(span)
    }
}

impl Default for AudioPace {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Source frames pushed into the graph per input chunk.
pub const STAGE_FEED_FRAMES: usize = 512;

/// The filter chain between `abuffer` and `abuffersink` for `speed` at `sample_rate`.
pub fn stage_filters(speed: Speed, preserve_pitch: bool, sample_rate: u32) -> String {
    let format =
        format!("aformat=sample_fmts=flt:channel_layouts=stereo:sample_rates={sample_rate}");
    if preserve_pitch {
        let s = f64::from(speed.percent()) / 100.0;
        let n = s.log2().abs().ceil().max(1.0);
        let factor = s.powf(1.0 / n);
        let mut chain = String::new();
        for _ in 0..n as usize {
            chain.push_str(&format!("atempo={factor:.17},"));
        }
        chain + &format
    } else {
        // Others than the rates divisible by 100 round to the nearest hertz (the drift check
        // of the caller re-anchors the voice if that ever adds up).
        let rate = (u64::from(sample_rate) * u64::from(speed.percent()) + 50) / 100;
        format!("asetrate={rate},aresample={sample_rate},{format}")
    }
}

fn stage_error(what: &str, err: impl std::fmt::Display) -> MediaError {
    MediaError::new(
        MediaErrorKind::UnsupportedCodec,
        format!("audio time stage: {what}: {err}"),
    )
}

/// One configured filter graph: interleaved stereo `f32` in, the same out.
struct Graph {
    graph: AVFilterGraph,
    sample_rate: u32,
    pts: i64,
}

impl Graph {
    fn new(speed: Speed, preserve_pitch: bool, sample_rate: u32) -> Result<Self, MediaError> {
        let graph = AVFilterGraph::new();
        let filter = |name: &CStr| {
            AVFilter::get_by_name(name)
                .ok_or_else(|| stage_error("filter missing", name.to_string_lossy()))
        };
        let abuffer = filter(c"abuffer")?;
        let abuffersink = filter(c"abuffersink")?;
        let args = CString::new(format!(
            "time_base=1/{sample_rate}:sample_rate={sample_rate}:sample_fmt=flt:channel_layout=stereo"
        ))
        .map_err(|err| stage_error("arguments", err))?;
        let spec = CString::new(stage_filters(speed, preserve_pitch, sample_rate))
            .map_err(|err| stage_error("filters", err))?;
        {
            let mut src = graph
                .create_filter_context(&abuffer, c"in", Some(&args))
                .map_err(|err| stage_error("abuffer", err))?;
            let mut sink = graph
                .create_filter_context(&abuffersink, c"out", None)
                .map_err(|err| stage_error("abuffersink", err))?;
            let outputs = AVFilterInOut::new(c"in", &mut src, 0);
            let inputs = AVFilterInOut::new(c"out", &mut sink, 0);
            graph
                .parse_ptr(&spec, Some(inputs), Some(outputs))
                .map_err(|err| stage_error("parse", err))?;
        }
        graph.config().map_err(|err| stage_error("config", err))?;
        Ok(Self {
            graph,
            sample_rate,
            pts: 0,
        })
    }

    /// Pushes source frames, then appends everything the graph has ready to `out`.
    fn push(&mut self, pcm: &[f32], out: &mut Vec<f32>) -> Result<(), MediaError> {
        let frames = pcm.len() / 2;
        if frames == 0 {
            return Ok(());
        }
        let mut frame = AVFrame::new();
        frame.set_format(ffi::AV_SAMPLE_FMT_FLT);
        frame.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
        frame.set_sample_rate(self.sample_rate as i32);
        frame.set_nb_samples(frames as i32);
        frame.set_pts(self.pts);
        frame
            .alloc_buffer()
            .map_err(|err| stage_error("frame", err))?;
        // SAFETY: the frame was just allocated for `frames` packed stereo f32 samples, so plane 0
        // holds at least `2 x frames` floats; `pcm` has as many.
        unsafe {
            std::ptr::copy_nonoverlapping(pcm.as_ptr(), frame.data[0].cast::<f32>(), 2 * frames);
        }
        self.graph
            .get_filter(c"in")
            .ok_or_else(|| stage_error("graph", "no source"))?
            .buffersrc_add_frame(Some(frame), None)
            .map_err(|err| stage_error("push", err))?;
        self.pts += frames as i64;
        loop {
            let pulled = self
                .graph
                .get_filter(c"out")
                .ok_or_else(|| stage_error("graph", "no sink"))?
                .buffersink_get_frame(None);
            let frame = match pulled {
                Ok(frame) => frame,
                Err(RsmpegError::BufferSinkDrainError | RsmpegError::BufferSinkEofError) => {
                    return Ok(());
                }
                Err(err) => return Err(stage_error("pull", err)),
            };
            let frames = frame.nb_samples.max(0) as usize;
            // SAFETY: the sink delivers packed stereo f32 (the `aformat` before it), so plane 0
            // holds `2 x nb_samples` floats.
            let pcm =
                unsafe { std::slice::from_raw_parts(frame.data[0].cast::<f32>(), 2 * frames) };
            out.extend_from_slice(pcm);
        }
    }
}

/// Output frames dropped after a reset: one `atempo` window (`sample_rate / 24` rounded up to a
/// power of two, 2048 at 48 kHz: its fade-in), or a few for the resampler.
fn drop_frames(preserve_pitch: bool, sample_rate: u32) -> usize {
    if preserve_pitch {
        (sample_rate as usize / 24).next_power_of_two()
    } else {
        64
    }
}

/// The lead of a graph kind in output frames (see the module docs), measured once per process
/// and cached: ten 1 kHz bursts with a 40 ms Hann envelope, 250 ms apart (3 s of source), go
/// through a fresh graph, and the lead is the mean distance of their energy centres before the
/// ideal `x / speed` (ten bursts average WSOLA's per-fragment jitter of about +-4 ms). It costs
/// 7 to 15 ms of CPU (dev build) the first time a speed is used (a speed slider drag measures
/// each new value once, on the feeder thread). The resampler has no lead (its filter delay is
/// compensated; `leads_are_small_and_resampling_has_none`), so it is not measured.
pub fn stage_lead(speed: Speed, preserve_pitch: bool, sample_rate: u32) -> Result<f64, MediaError> {
    if !preserve_pitch {
        return Ok(0.0);
    }
    /// Leads by (speed percent, sample rate).
    type Leads = Mutex<HashMap<(u16, u32), f64>>;
    static LEADS: OnceLock<Leads> = OnceLock::new();
    let key = (speed.percent(), sample_rate);
    let leads = LEADS.get_or_init(Mutex::default);
    if let Some(lead) = leads.lock().ok().and_then(|m| m.get(&key).copied()) {
        return Ok(lead);
    }
    let lead = measure_lead(speed, preserve_pitch, sample_rate)?;
    if let Ok(mut map) = leads.lock() {
        map.insert(key, lead);
    }
    Ok(lead)
}

fn measure_lead(speed: Speed, preserve_pitch: bool, sample_rate: u32) -> Result<f64, MediaError> {
    let sr = sample_rate as usize;
    let s = f64::from(speed.percent()) / 100.0;
    let spacing = sr / 4;
    let burst = sr / 25;
    let first = sr / 2;
    let bursts = 10;
    let len = first + bursts * spacing;
    let mut source = vec![0.0f32; 2 * len];
    for b in 0..bursts {
        let at = first + b * spacing;
        for j in 0..burst {
            let phase = j as f64 / burst as f64;
            let envelope = 0.5 - 0.5 * (2.0 * PI * phase).cos();
            let v = (envelope * (2.0 * PI * 1000.0 * j as f64 / sr as f64).sin()) as f32;
            source[2 * (at + j)] = v;
            source[2 * (at + j) + 1] = v;
        }
    }
    let mut graph = Graph::new(speed, preserve_pitch, sample_rate)?;
    let mut out = Vec::new();
    for chunk in source.chunks(2 * STAGE_FEED_FRAMES) {
        graph.push(chunk, &mut out)?;
    }
    // Flush the tail with silence.
    let silence = vec![0.0f32; 2 * STAGE_FEED_FRAMES];
    let wanted = ((len as f64 / s) as usize + sr / 10) * 2;
    while out.len() < wanted {
        graph.push(&silence, &mut out)?;
    }
    let mut offsets = Vec::with_capacity(bursts);
    for b in 0..bursts {
        let centre = (first + b * spacing) as f64 + burst as f64 / 2.0;
        let ideal = centre / s;
        let half = spacing as f64 / s / 2.0;
        let lo = (ideal - half).max(0.0) as usize;
        let hi = ((ideal + half) as usize).min(out.len() / 2);
        let (mut energy, mut moment) = (0.0f64, 0.0f64);
        for i in lo..hi {
            let v = f64::from(out[2 * i]);
            energy += v * v;
            moment += v * v * i as f64;
        }
        if energy > 0.0 {
            offsets.push(ideal - moment / energy);
        }
    }
    if offsets.is_empty() {
        return Err(stage_error("calibration", "no output"));
    }
    Ok(offsets.iter().sum::<f64>() / offsets.len() as f64)
}

/// A per-voice `libavfilter` graph that turns source PCM into output PCM at `speed`, both
/// interleaved stereo `f32` at `sample_rate`. Owned and used on one thread.
pub struct TimeStage {
    graph: Graph,
    speed: Speed,
    preserve_pitch: bool,
    sample_rate: u32,
    /// Output frames early (module docs), rounded.
    lead: i64,
    /// Source frames to feed in front of the wanted source time after a reset.
    preroll: usize,
    /// Output frames still to drop after the last reset.
    drop: usize,
    /// Output frames not taken yet, from `fifo_read` on.
    fifo: Vec<f32>,
    fifo_read: usize,
    /// Source frames pushed and output frames the graph emitted (the dropped ones included)
    /// since the last reset.
    fed: i64,
    emitted: i64,
    /// Scratch for one input chunk.
    chunk: Vec<f32>,
}

impl std::fmt::Debug for TimeStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeStage")
            .field("speed", &self.speed)
            .field("preserve_pitch", &self.preserve_pitch)
            .field("sample_rate", &self.sample_rate)
            .field("lead", &self.lead)
            .field("fed", &self.fed)
            .field("emitted", &self.emitted)
            .finish_non_exhaustive()
    }
}

impl TimeStage {
    /// Builds the stage for `speed` at `sample_rate` (and measures the graph kind's lead the
    /// first time it is used).
    pub fn new(speed: Speed, preserve_pitch: bool, sample_rate: u32) -> Result<Self, MediaError> {
        let sample_rate = sample_rate.max(1);
        let lead = stage_lead(speed, preserve_pitch, sample_rate)?.round() as i64;
        let drop = drop_frames(preserve_pitch, sample_rate);
        let p = i64::from(speed.percent());
        // Output frame `drop` carries source frame `(drop + lead) x speed` after the start of
        // the feed: that many source frames go in front of the wanted time.
        let preroll = usize::try_from(((drop as i64 + lead) * p + 50) / 100).unwrap_or(0);
        Ok(Self {
            graph: Graph::new(speed, preserve_pitch, sample_rate)?,
            speed,
            preserve_pitch,
            sample_rate,
            lead,
            preroll,
            drop,
            fifo: Vec::new(),
            fifo_read: 0,
            fed: 0,
            emitted: 0,
            chunk: vec![0.0; 2 * STAGE_FEED_FRAMES],
        })
    }

    pub fn speed(&self) -> Speed {
        self.speed
    }

    pub fn preserve_pitch(&self) -> bool {
        self.preserve_pitch
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Whether this stage plays `speed` with `preserve_pitch` at `sample_rate`.
    pub fn plays(&self, speed: Speed, preserve_pitch: bool, sample_rate: u32) -> bool {
        self.speed == speed
            && self.preserve_pitch == preserve_pitch
            && self.sample_rate == sample_rate
    }

    /// Source frames to feed in front of the wanted source time after a reset (in the reading
    /// direction: below it forward, above it reversed).
    pub fn preroll_frames(&self) -> usize {
        self.preroll
    }

    /// The source time span of [`Self::preroll_frames`].
    pub fn preroll(&self) -> Time {
        sample_offset(self.preroll as i64, self.sample_rate)
    }

    /// Starts over (a seek, a jump, a re-anchor): a fresh graph and an empty FIFO. Feed from
    /// [`Self::preroll_frames`] before the wanted source time next.
    pub fn reset(&mut self) -> Result<(), MediaError> {
        self.graph = Graph::new(self.speed, self.preserve_pitch, self.sample_rate)?;
        self.drop = drop_frames(self.preserve_pitch, self.sample_rate);
        self.fifo.clear();
        self.fifo_read = 0;
        self.fed = 0;
        self.emitted = 0;
        Ok(())
    }

    /// Source frames inside the graph that no emitted output frame carries yet:
    /// `fed - (emitted + lead) x speed`. It stays within about one window times the speed plus
    /// one feed chunk while the stage keeps time; growing or shrinking past that means its
    /// output count drifted (the caller re-anchors).
    pub fn backlog_frames(&self) -> i64 {
        let p = i64::from(self.speed.percent());
        self.fed - (self.emitted + self.lead) * p / 100
    }

    /// The largest [`Self::backlog_frames`] of a stage that keeps time, plus `slack` (at the
    /// stage's rate, source frames).
    pub fn backlog_limit(&self, slack: usize) -> i64 {
        let p = i64::from(self.speed.percent());
        let window = drop_frames(self.preserve_pitch, self.sample_rate) as i64;
        // Every chained atempo holds up to a window of its input.
        let chain = stage_filters(self.speed, self.preserve_pitch, self.sample_rate)
            .matches("atempo")
            .count()
            .max(1) as i64;
        (chain * window * p.max(100)) / 100 + STAGE_FEED_FRAMES as i64 + slack as i64
    }

    fn buffered(&self) -> usize {
        (self.fifo.len() - self.fifo_read) / 2
    }

    /// Pushes source frames (interleaved stereo) into the graph.
    pub fn push(&mut self, pcm: &[f32]) -> Result<(), MediaError> {
        let before = self.fifo.len();
        self.graph.push(pcm, &mut self.fifo)?;
        self.fed += (pcm.len() / 2) as i64;
        let new = (self.fifo.len() - before) / 2;
        self.emitted += new as i64;
        // Drop the preroll's share of the output.
        let skip = self.drop.min(new);
        if skip > 0 {
            self.fifo.drain(before..before + 2 * skip);
            self.drop -= skip;
        }
        Ok(())
    }

    /// Makes sure at least `frames` output frames are ready, pulling source frames from `feed`
    /// (which fills its slice with the next source frames) as needed.
    pub fn fill(
        &mut self,
        frames: usize,
        feed: &mut dyn FnMut(&mut [f32]) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        while self.buffered() < frames {
            let mut chunk = std::mem::take(&mut self.chunk);
            let fed = feed(&mut chunk).and_then(|()| self.push(&chunk));
            self.chunk = chunk;
            fed?;
        }
        Ok(())
    }

    /// Fills `out` (interleaved stereo) with the next output frames, pulling source frames from
    /// `feed` as needed.
    pub fn take(
        &mut self,
        out: &mut [f32],
        feed: &mut dyn FnMut(&mut [f32]) -> Result<(), MediaError>,
    ) -> Result<(), MediaError> {
        let frames = out.len() / 2;
        self.fill(frames, feed)?;
        let from = self.fifo_read;
        out[..2 * frames].copy_from_slice(&self.fifo[from..from + 2 * frames]);
        self.fifo_read += 2 * frames;
        if self.fifo_read >= 1 << 14 {
            self.fifo.drain(..self.fifo_read);
            self.fifo_read = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn speed(p: u16) -> Speed {
        Speed::new(p).unwrap()
    }

    #[test]
    fn filter_chains_keep_each_atempo_within_half_to_double() {
        assert_eq!(
            stage_filters(speed(200), true, SR),
            "atempo=2.00000000000000000,\
             aformat=sample_fmts=flt:channel_layouts=stereo:sample_rates=48000"
        );
        let count = |p| stage_filters(speed(p), true, SR).matches("atempo").count();
        assert_eq!(count(150), 1);
        assert_eq!(count(50), 1);
        assert_eq!(count(400), 2);
        assert_eq!(count(300), 2);
        assert_eq!(count(40), 2);
        assert_eq!(count(10), 4);
        assert_eq!(
            stage_filters(speed(150), false, 44_100),
            "asetrate=66150,aresample=44100,\
             aformat=sample_fmts=flt:channel_layouts=stereo:sample_rates=44100"
        );
    }

    #[test]
    fn paces() {
        assert!(AudioPace::NORMAL.sounds() && !AudioPace::NORMAL.stretches());
        let at = |p| AudioPace {
            speed: speed(p),
            ..AudioPace::NORMAL
        };
        assert!(!at(401).sounds());
        assert!(at(400).sounds() && at(400).stretches());
        let one = AudioPace {
            preserve_pitch: false,
            ..AudioPace::NORMAL
        };
        assert_eq!(one.normalized(), AudioPace::NORMAL);
        assert_eq!(at(200).source_span(Time(1_000)), Time(2_000));
    }

    #[test]
    fn every_graph_kind_builds() {
        for p in [10, 50, 75, 150, 200, 400] {
            for pitch in [true, false] {
                for rate in [44_100, 48_000, 96_000] {
                    TimeStage::new(speed(p), pitch, rate)
                        .unwrap_or_else(|err| panic!("{p}% pitch {pitch} at {rate}: {err}"));
                }
            }
        }
    }

    #[test]
    fn leads_are_small_and_resampling_has_none() {
        for p in [50, 100, 150, 200, 400] {
            let lead = stage_lead(speed(p), true, SR).unwrap();
            // 7 to 27 ms (module docs), in output frames.
            assert!((200.0..1_400.0).contains(&lead), "{p}%: lead {lead}");
        }
        for p in [50, 150, 200] {
            let lead = measure_lead(speed(p), false, SR).unwrap();
            assert!(lead.abs() < 2.0, "{p}% resampled: lead {lead}");
        }
    }
}
