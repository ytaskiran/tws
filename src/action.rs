pub enum Action {
    // Navigation
    Quit,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    ToggleExpand,

    // CRUD
    StartAdd,
    StartRename,
    StartDelete,
    ConfirmInput(String),
    ConfirmDelete,
    CancelModal,

    // Text input
    InputChar(char),
    InputBackspace,

    None,
}
