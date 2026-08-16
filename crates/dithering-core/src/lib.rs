//! Core dithering pipeline. Nothing here yet.

/// Placeholder until the pipeline lands.
pub fn hello() -> String {
    "Hello, world! from dithering-core".to_string()
}

#[cfg(test)]
mod tests {
    use super::hello;

    #[test]
    fn hello_names_the_crate() {
        assert!(hello().contains("dithering-core"));
    }
}
