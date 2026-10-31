//! Draft connection settings. Never serialize these into public configuration.
use zeroize::Zeroizing;

#[derive(Default)]
pub(crate) struct CloudSetup {
    pub(crate) username: String,
    pub(crate) space_name: String,
    pub(crate) base_url: String,
    pub(crate) bucket: String,
    pub(crate) access_key: Zeroizing<String>,
    pub(crate) secret_key: Zeroizing<String>,
}

impl CloudSetup {
    /// Syntax validation only: no network requests or credential verification.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        for (name, error) in [
            (
                &self.username,
                "Введите имя пользователя (до 256 байт, без управляющих символов)",
            ),
            (
                &self.space_name,
                "Введите имя хранилища (до 256 байт, без управляющих символов)",
            ),
        ] {
            if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(error);
            }
        }
        let endpoint = url::Url::parse(&self.base_url)
            .map_err(|_| "Введите Base URL в формате https://адрес")?;
        if self.base_url.trim() != self.base_url
            || self.base_url.chars().any(char::is_whitespace)
            || endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
        {
            return Err(
                "Base URL должен содержать HTTPS-адрес gateway без логина, пути, query и fragment",
            );
        }
        let bucket = self.bucket.as_bytes();
        if !(3..=63).contains(&bucket.len())
            || !bucket.first().is_some_and(u8::is_ascii_alphanumeric)
            || !bucket.last().is_some_and(u8::is_ascii_alphanumeric)
            || !bucket
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        {
            return Err("Bucket: 3–63 символа a–z, 0–9 и дефис; начало и конец — буква или цифра");
        }
        for (key, error) in [
            (
                &self.access_key,
                "Введите Access Key без пробелов и управляющих символов",
            ),
            (
                &self.secret_key,
                "Введите Secret Key без пробелов и управляющих символов",
            ),
        ] {
            if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(error);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/cloud_setup.rs"]
mod tests;
