//! Video export (`docs/design/export.md`): the pure export plan, the output-file rules, encoder
//! candidates and the session probe, the NV12 hand-off, audio and the MP4 muxer.
//!
//! - [`plan`]: output size, frame rate, frame and sample counts, bitrate and size estimate (pure).
//! - [`output`]: output path rules, the partial file name and the overwrite check (§5.2).
//! - [`encoders`]: the encoder candidate table, option sets, test-encode probe and
//!   `CUTFRAME_ENCODERS` (§4.4).
//! - [`nv12`]: the canvas to BT.709 limited NV12 on the GPU, the staging ring and the copy into
//!   encoder frames (§4.3).
//! - [`audio`]: AAC-LC from the shared `mix_block` at the project rate (§4.5).
//! - [`mux`]: the MP4 muxer with `+faststart`, colour tags and `hvc1` (§4.6).
//! - [`run`]: the export loop: render, NV12, encode, mux, encoder fallback, cancel, throttle,
//!   disk floor, partial file and rename, the error taxonomy (§4.2, §5).

pub mod audio;
pub mod encoders;
pub mod mux;
pub mod nv12;
pub mod output;
pub mod plan;
pub mod run;
