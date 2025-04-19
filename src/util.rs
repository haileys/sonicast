use std::io;
use std::error::Error as StdError;

pub fn broken_pipe(err: &(dyn StdError + 'static)) -> bool {
    io_error(err).map(io::Error::kind) == Some(io::ErrorKind::BrokenPipe)
}

pub fn io_error<'err>(err: &'err (dyn StdError + 'static)) -> Option<&'err io::Error> {
    if let Some(io) = err.downcast_ref() {
        return Some(*io);
    }

    io_error(err.source()?)
}
