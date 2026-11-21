//! The CAM engine for SolidCraft's Manufacture workspace: the [`Toolpath`] a machining operation
//! produces, its [`Move`]s, and the units and tolerances they are measured in.
//!
//! The crate knows nothing of documents, designs or the UI. Its inputs are plain data and its
//! outputs are toolpaths and reports, so it can be tested on its own. Later phases add the
//! strategies that compute toolpaths and the post-processor here. [`read_gcode`] reads G-code
//! back into a toolpath, so every program we write can be checked against its source.
//!
//! Units are millimetres, minutes and radians internally: positions in mm, feeds in mm/min,
//! spindle speeds in rev/min, dwells in seconds. [`LengthUnit`] converts at the edges only.
//!
//! Every input is checked ([`Toolpath::validate`], [`Tolerance::new`]) and every size is capped
//! ([`MAX_MOVES`]): a bad toolpath is an error, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod gcode_reader;
mod post;
mod toolpath;
mod units;

pub use gcode_reader::{MAX_LINE_CHARS, MAX_LINES, ReadBack, ReadSettings, read as read_gcode};

pub use post::{Arcs, BUILTIN_POSTS, Comments, LineNumbers, MachineKind, Modal, Numbers, POST_FORMAT, Post, Posted, ProgramContext, Sequences};
pub use toolpath::{
    ARC_RADIUS_TOLERANCE, ArcDirection, Coolant, MAX_DWELL_SECONDS, MAX_FEED, MAX_MOVES, MAX_SPINDLE_RPM, MAX_TOOL_NUMBER, Move, Plane,
    SpindleDirection, Stats, Toolpath,
};
pub use units::{DEFAULT_TOLERANCE, LengthUnit, MAX_COORDINATE, MM_PER_INCH, Tolerance};

/// Why a toolpath or a value was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// A move is not usable: the index of the move and why.
    #[error("move {index}: {reason}")]
    BadMove { index: usize, reason: String },
    /// The toolpath has more moves than [`MAX_MOVES`].
    #[error("too many moves: {count} (at most {max})")]
    TooManyMoves { count: usize, max: usize },
    /// A G-code program could not be read: the 1-based line and why.
    #[error("line {line}: {message}")]
    Gcode { line: usize, message: String },
    /// A post file is not usable.
    #[error("post: {0}")]
    BadPost(String),
    /// A written program did not read back as the moves it was written for.
    #[error("post read-back, move {index}: {message}")]
    PostMismatch { index: usize, message: String },
    /// A value outside its allowed range.
    #[error("{0}")]
    BadValue(String),
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;
