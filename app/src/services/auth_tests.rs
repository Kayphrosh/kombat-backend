#[cfg(test)]
mod tests {
    use super::super::auth::verify_jwt_get_wallet;
    use jsonwebtoken::{EncodingKey, Header};

    #[test]
    fn test_verify_jwt_get_wallet() {
        let secret = "test-secret-123";
        #[derive(serde::Serialize)]
        struct Claims<'a> { wallet: &'a str, exp: usize }
        let exp = (chrono::Utc::now() + chrono::Duration::seconds(60)).timestamp() as usize;
        let claims = Claims { wallet: "Wallet123", exp };
        let token = jsonwebtoken::encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap();
        let wallet = verify_jwt_get_wallet(&token, secret).unwrap();
        assert_eq!(wallet, "Wallet123");
    }
}
