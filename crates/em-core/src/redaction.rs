use std::fmt;

pub struct Redacted<'a, T>(pub &'a T);

impl<T> fmt::Debug for Redacted<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl<T> fmt::Display for Redacted<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_formats_inner_value() {
        let secret = "password";
        assert_eq!(format!("{}", Redacted(&secret)), "[REDACTED]");
        assert_eq!(format!("{:?}", Redacted(&secret)), "[REDACTED]");
    }
}
