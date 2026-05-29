pub mod pipe;
mod process;
pub mod process_group;
pub mod pty;
#[cfg(test)]
mod tests;

pub const DEFAULT_OUTPUT_BYTES_CAP: usize = 1024 * 1024;

pub use pipe::spawn_process as spawn_pipe_process;

pub use pipe::spawn_process_no_stdin as spawn_pipe_process_no_stdin;

pub use process::ProcessDriver;

pub use process::ProcessHandle;

pub use process::SpawnedProcess;

pub use process::TerminalSize;

pub use process::combine_output_receivers;

pub use process::spawn_from_driver;

pub type ExecCommandSession = ProcessHandle;

pub type SpawnedPty = SpawnedProcess;

pub use pty::conpty_supported;

pub use pty::spawn_process as spawn_pty_process;
