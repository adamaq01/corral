#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub subject: String,
    pub preferred_username: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
}
