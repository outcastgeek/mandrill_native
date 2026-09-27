use mandrill_effect::{Defect, DefectKind, LibraryDefect};
const DEFECT: &Defect = &Defect::library(LibraryDefect::RunDriverDropped);
const KIND: DefectKind = DEFECT.kind();
fn main() { assert_eq!(KIND, DefectKind::Library(LibraryDefect::RunDriverDropped)); }
