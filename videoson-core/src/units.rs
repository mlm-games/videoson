#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBase {
    pub num: u32,
    pub den: u32,
}

impl TimeBase {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }
}
