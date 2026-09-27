use mandrill_effect::{Defect, DefectKind, LibraryDefect};

fn main() {
    let defect = Defect::from(LibraryDefect::ResourceScopeEnded);
    match defect.kind() {
        DefectKind::Library(LibraryDefect::ResourceScopeEnded) => {}
        DefectKind::Custom | DefectKind::CallbackPanic | DefectKind::Library(_) => {}
    }
    assert_ne!(defect, Defect::new(defect.message()));
}
