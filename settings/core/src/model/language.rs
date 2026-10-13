#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AppLanguage {
    #[default]
    SystemDefault,
    Japanese,
    English,
}
