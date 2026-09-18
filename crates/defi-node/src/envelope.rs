use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    pub simulated: bool,
    pub custodial: bool,
    pub settlement: &'static str,
    #[serde(flatten)]
    pub data: T,
}

impl<T> Envelope<T> {
    pub fn wrap(data: T) -> Self {
        Self {
            simulated: true,
            custodial: false,
            settlement: "none",
            data,
        }
    }
}
