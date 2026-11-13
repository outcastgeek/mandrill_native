pub mod neomorphism;
pub mod shared;
pub mod unstyled;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentInfo {
    pub family: &'static str,
    pub module: &'static str,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileSource {
    pub family: &'static str,
    pub module: &'static str,
    pub path: &'static str,
    pub source: &'static str,
}

#[must_use]
pub fn all_components() -> Vec<ComponentInfo> {
    let mut components = unstyled::all_components();
    components.extend(neomorphism::all_components());
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_components_combines_unstyled_and_neomorphism_registries() {
        let unstyled_components = unstyled::all_components();
        let neomorphism_components = neomorphism::all_components();
        let all = all_components();

        assert_eq!(
            all.len(),
            unstyled_components.len() + neomorphism_components.len()
        );
        assert!(all.iter().any(|component| component.family == "unstyled"));
        assert!(
            all.iter()
                .any(|component| component.family == "neomorphism")
        );
        assert!(all.iter().any(|component| component.name == "Button"));
        assert!(all.iter().any(|component| component.name == "ChevronLeft"));
    }
}
