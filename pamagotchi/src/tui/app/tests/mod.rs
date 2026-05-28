use super::*;

#[test]
fn vertical_cursor_movement_preserves_utf8_boundaries() {
    let mut app = App::new(0);
    app.input = "ééé\nabcd".into();
    app.cursor = "ééé\na".len();

    app.move_cursor_up();

    assert!(app.input.is_char_boundary(app.cursor));
    assert_eq!(app.cursor, "é".len());

    let mut app = App::new(0);
    app.input = "abcd\nééé".into();
    app.cursor = "a".len();

    app.move_cursor_down();

    assert!(app.input.is_char_boundary(app.cursor));
    assert_eq!(app.cursor, "abcd\né".len());
}

#[test]
fn visual_cursor_helpers_tolerate_non_boundary_offsets() {
    let text = "é🙂\nمرحبا";

    assert!(!text.is_char_boundary(1));
    assert_eq!(visual_cursor_x(text, 1, 80), 0);
    assert_eq!(visual_cursor_y(text, 1, 80), 0);
}

#[test]
fn gateway_var_editing_clamps_invalid_cursor_before_insert() {
    let mut app = App::new(0);
    app.gateway_var_input = "éx".into();
    app.gateway_var_cursor = 1;

    app.insert_gateway_var_char('🙂');

    assert_eq!(app.gateway_var_input, "🙂éx");
    assert!(
        app.gateway_var_input
            .is_char_boundary(app.gateway_var_cursor)
    );
}
