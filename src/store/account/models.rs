use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    #[serde(rename = "refreshToken")]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PublicKey {
    #[serde(rename = "publicKey")]
    pub public_key: String,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub username: String,
}
