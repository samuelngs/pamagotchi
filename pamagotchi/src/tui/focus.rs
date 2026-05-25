#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusId {
    Input,
    Quit,
    Gateway,
}

const ORDER: &[FocusId] = &[FocusId::Input, FocusId::Quit, FocusId::Gateway];

pub struct FocusManager {
    index: usize,
}

impl FocusManager {
    pub fn new() -> Self {
        Self { index: 0 }
    }

    pub fn current(&self) -> FocusId {
        ORDER[self.index]
    }

    pub fn is(&self, id: FocusId) -> bool {
        self.current() == id
    }

    pub fn next(&mut self) {
        self.index = (self.index + 1) % ORDER.len();
    }

    pub fn prev(&mut self) {
        self.index = (self.index + ORDER.len() - 1) % ORDER.len();
    }

    pub fn set(&mut self, id: FocusId) {
        if let Some(pos) = ORDER.iter().position(|&x| x == id) {
            self.index = pos;
        }
    }
}
