use core::{
    fmt::{self, Write},
    str,
};

/// A buffer of fixed length into which strings can be written.
pub struct StringBuffer<const N: usize> {
    buffer: [u8; N],
    length: usize,
}

impl<const N: usize> StringBuffer<N> {
    pub const fn new() -> Self {
        Self {
            buffer: [0; N],
            length: 0,
        }
    }

    pub fn to_str(&self) -> Result<&str, str::Utf8Error> {
        str::from_utf8(&self.buffer[..self.length])
    }
}

impl<const N: usize> Write for StringBuffer<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.length + s.len() > N {
            return Err(fmt::Error);
        }
        self.buffer[self.length..self.length + s.len()].copy_from_slice(s.as_bytes());
        self.length += s.len();
        Ok(())
    }
}
