#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusId {
    Input,
    Quit,
    Settings,
    GatewayList,
    GatewayBack,
    GatewayAdd,
    GatewayAddKind,
    GatewayDetailBack,
    GatewayDetailRemove,
    GatewayDetailRestart,
}

const ORDER: &[FocusId] = &[FocusId::Input, FocusId::Quit, FocusId::Settings];

pub struct FocusManager {
    current: FocusId,
}

impl FocusManager {
    pub fn new() -> Self {
        Self {
            current: FocusId::Input,
        }
    }

    pub fn current(&self) -> FocusId {
        self.current
    }

    pub fn is(&self, id: FocusId) -> bool {
        self.current() == id
    }

    pub fn next(&mut self) {
        self.next_in(ORDER);
    }

    pub fn prev(&mut self) {
        self.prev_in(ORDER);
    }

    pub fn set(&mut self, id: FocusId) {
        self.current = id;
    }

    pub fn next_in(&mut self, order: &[FocusId]) {
        if order.is_empty() {
            return;
        }
        let pos = order.iter().position(|&x| x == self.current).unwrap_or(0);
        self.current = order[(pos + 1) % order.len()];
    }

    pub fn prev_in(&mut self, order: &[FocusId]) {
        if order.is_empty() {
            return;
        }
        let pos = order.iter().position(|&x| x == self.current).unwrap_or(0);
        self.current = order[(pos + order.len() - 1) % order.len()];
    }
}
