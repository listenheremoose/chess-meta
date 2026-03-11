pub mod controls;
pub mod move_table;
pub mod progress;
pub mod tree_view;

/// Dark theme colors.
pub mod colors {
    use iced::Color;

    pub const BACKGROUND: Color = Color::from_rgb(
        0x1E as f32 / 255.0,
        0x1E as f32 / 255.0,
        0x1E as f32 / 255.0,
    );
    pub const SURFACE: Color = Color::from_rgb(
        0x2D as f32 / 255.0,
        0x2D as f32 / 255.0,
        0x2D as f32 / 255.0,
    );
    pub const TEXT: Color = Color::from_rgb(
        0xE0 as f32 / 255.0,
        0xE0 as f32 / 255.0,
        0xE0 as f32 / 255.0,
    );
    pub const TEXT_DIM: Color = Color::from_rgb(
        0x80 as f32 / 255.0,
        0x80 as f32 / 255.0,
        0x80 as f32 / 255.0,
    );
    pub const ACCENT: Color = Color::from_rgb(
        0x4E as f32 / 255.0,
        0xC9 as f32 / 255.0,
        0xB0 as f32 / 255.0,
    );
    pub const GREEN: Color = Color::from_rgb(
        0x4E as f32 / 255.0,
        0xC9 as f32 / 255.0,
        0x4E as f32 / 255.0,
    );
    pub const RED: Color = Color::from_rgb(
        0xE0 as f32 / 255.0,
        0x50 as f32 / 255.0,
        0x50 as f32 / 255.0,
    );
    pub const BLUE: Color = Color::from_rgb(
        0x50 as f32 / 255.0,
        0x90 as f32 / 255.0,
        0xE0 as f32 / 255.0,
    );
    pub const ORANGE: Color = Color::from_rgb(
        0xE0 as f32 / 255.0,
        0x90 as f32 / 255.0,
        0x50 as f32 / 255.0,
    );
}
