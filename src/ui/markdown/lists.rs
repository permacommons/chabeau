pub(super) const MAX_LIST_HANGING_INDENT_WIDTH: usize = 32;

#[derive(Clone, Debug)]
pub(super) enum ListKind {
    Unordered,
    Ordered(u64),
}
