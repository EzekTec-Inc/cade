#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamMode {
    Coordinate,
    Route,
    Broadcast,
    Tasks,
}
impl TeamMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Coordinate => "coordinate",
            Self::Route => "route",
            Self::Broadcast => "broadcast",
            Self::Tasks => "tasks",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "coordinate" => Some(Self::Coordinate),
            "route" | "router" => Some(Self::Route),
            "broadcast" => Some(Self::Broadcast),
            "tasks" | "task" => Some(Self::Tasks),
            _ => None,
        }
    }
}
impl std::fmt::Display for TeamMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl Default for TeamMode {
    fn default() -> Self {
        Self::Coordinate
    }
}
