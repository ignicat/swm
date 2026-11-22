pub(crate) mod camera;
pub(crate) mod maximize;
pub(crate) mod window;

/// Identifies the output whose presentation state an entry belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PresentationScope {
    pub(crate) output: String,
}

impl PresentationScope {
    pub(crate) fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_distinguished_by_output() {
        let first = PresentationScope::new("DP-1");
        let second = PresentationScope::new("DP-2");
        assert_ne!(first, second);
        assert_eq!(first, PresentationScope::new("DP-1"));
    }
}
