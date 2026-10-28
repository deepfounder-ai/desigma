//! Every visual constant of the editor chrome lives here.
//! Existing values retain the legacy Figma UI3 measurements in `assets/ui-spec.json`;
//! new editor work targets Desigma's own visual design, not Figma screenshot parity.
//! Nothing outside this module may hardcode colors, sizes, radii or fonts.

use egui::{Color32, CornerRadius, FontFamily, FontId};

/// Opaque color from `0xRRGGBB`.
pub const fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Color from `0xRRGGBB` plus alpha 0..=255 (premultiplied internally).
pub const fn rgba(c: u32, a: u8) -> Color32 {
    let r = ((c >> 16) & 0xFF) * a as u32 / 255;
    let g = ((c >> 8) & 0xFF) * a as u32 / 255;
    let b = (c & 0xFF) * a as u32 / 255;
    Color32::from_rgba_premultiplied(r as u8, g as u8, b as u8, a)
}

// ---------------------------------------------------------------- colors
pub const BG: Color32 = rgb(0x2C2C2C);
pub const BG_SECONDARY: Color32 = rgb(0x383838);
pub const BG_TERTIARY: Color32 = rgb(0x444444);
pub const BG_HOVER: Color32 = rgb(0x383838);
pub const BG_PRESSED: Color32 = rgb(0x444444);
pub const BG_MENU: Color32 = rgb(0x1E1E1E);
pub const BG_SELECTED: Color32 = rgb(0x394360);
pub const BG_SELECTED_HOVER: Color32 = rgb(0x4A5878);
pub const BG_SELECTED_SECONDARY: Color32 = rgb(0x32394D);
pub const BG_BRAND: Color32 = rgb(0x0C8CE9);
pub const BG_BRAND_HOVER: Color32 = rgb(0x0A6DC2);
pub const BG_BRAND_PRESSED: Color32 = rgb(0x105CAD);
pub const BG_TOOLBAR: Color32 = rgb(0x2C2C2C);
pub const BG_TOOLBAR_HOVER: Color32 = rgb(0x111111);
pub const BG_TOOLBAR_SELECTED: Color32 = rgb(0x0C8CE9);
pub const BG_TOOLBAR_MODE_SWITCHER: Color32 = rgb(0x444444);
pub const BG_SELECTED_AREA: Color32 = rgba(0x0D99FF, 0x33);
pub const BORDER: Color32 = rgb(0x444444);
pub const BORDER_MENU: Color32 = rgb(0x383838);
pub const BORDER_SELECTED: Color32 = rgb(0x0C8CE9);
pub const BORDER_TRANSLUCENT: Color32 = rgba(0xFFFFFF, 0x1A);
pub const TEXT: Color32 = rgb(0xFFFFFF);
pub const TEXT_SECONDARY: Color32 = rgba(0xFFFFFF, 0xB2);
pub const TEXT_TERTIARY: Color32 = rgba(0xFFFFFF, 0x66);
pub const TEXT_DISABLED: Color32 = rgba(0xFFFFFF, 0x66);
pub const TEXT_BRAND: Color32 = rgb(0x7CC4F8);
pub const TEXT_COMPONENT: Color32 = rgb(0xD1A8FF);
pub const ICON: Color32 = rgb(0xFFFFFF);
pub const ICON_SECONDARY: Color32 = rgba(0xFFFFFF, 0xB2);
pub const ICON_TERTIARY: Color32 = rgba(0xFFFFFF, 0x66);
pub const ICON_SELECTED: Color32 = rgb(0x7CC4F8);
pub const DANGER: Color32 = rgb(0xF24822);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const BG_ELEVATED: Color32 = rgb(0x1E1E1E);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const BG_MENU_HOVER: Color32 = rgb(0x2C2C2C);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const BG_DISABLED: Color32 = rgb(0x757575);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const BG_TRANSPARENT_HOVER: Color32 = rgba(0xFFFFFF, 0x0D);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const BORDER_STRONG: Color32 = rgba(0xFFFFFF, 0xE5);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const COMPONENT: Color32 = rgb(0x9747FF);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const TEXT_ON_LIGHT_CANVAS: Color32 = rgba(0x000000, 0xE5);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const TEXT_ON_LIGHT_CANVAS_SECONDARY: Color32 = rgba(0x000000, 0x4D);
#[allow(dead_code)] // spec colour, kept for parity with ui-spec.json
pub const TEXT_ON_DARK_CANVAS: Color32 = rgba(0xFFFFFF, 0xE5);
pub const SHADOW_COLOR: Color32 = rgba(0x000000, 0x59);
pub const SHADOW_COLOR_STRONG: Color32 = rgba(0x000000, 0x80);
/// Elevation 400: drop shadow colour, 0.5px inset ring (#ffffff59 at half coverage) and top edge.
pub const ELEVATION_400_SHADOW: Color32 = rgba(0x000000, 0x59);
pub const ELEVATION_400_RING: Color32 = rgba(0xFFFFFF, 0x16);
pub const ELEVATION_400_TOP: Color32 = rgba(0xFFFFFF, 0x0A);
pub const INNER_HIGHLIGHT: Color32 = rgba(0xFFFFFF, 0x14);
pub const LOGO_GRADIENT_A: Color32 = rgb(0x0C8CE9);
pub const CHECKER_A: Color32 = rgb(0xFFFFFF);
pub const CHECKER_B: Color32 = rgb(0xCCCCCC);

// canvas overlays
pub const CANVAS_SELECTION: Color32 = rgb(0x0D99FF);
pub const CANVAS_COMPONENT: Color32 = rgb(0x9747FF);
pub const CANVAS_HANDLE_FILL: Color32 = rgb(0xFFFFFF);
pub const CANVAS_SIZE_BADGE_TEXT: Color32 = rgb(0xFFFFFF);
pub const CANVAS_LABEL: Color32 = rgba(0x000000, 0x80);
pub const CANVAS_LABEL_DARK: Color32 = rgba(0xFFFFFF, 0x99);
pub const CANVAS_MARQUEE_FILL: Color32 = BG_SELECTED_AREA;
pub const GUIDE_COLOR: Color32 = rgb(0xF24E7B);
pub const GUIDE_TEXT: Color32 = rgb(0xFFFFFF);
pub const PEN_POINT: Color32 = rgb(0xFFFFFF);
pub const DROP_INDICATOR: Color32 = rgb(0x0C8CE9);
pub const LIGHT_BG_THRESHOLD: f32 = 0.5;

// ---------------------------------------------------------------- fonts
pub const FONT_SIZE_SMALL: f32 = 9.0;
pub const FONT_SIZE_BODY: f32 = 11.0;
pub const FONT_SIZE_LARGE: f32 = 13.0;
pub const FONT_SIZE_TITLE: f32 = 20.0;
pub const CHEVRON_SMALL: f32 = 10.0;
pub const FONT_SIZE_LABEL_CANVAS: f32 = 11.0;
pub const FAMILY_STRONG: &str = "desigma-strong";
pub const UI_FONT_FAMILY: &str = "Inter";
/// Variable-font instances of the bundled Inter (ui-spec font.weight_default / weight_strong).
pub const UI_FONT_WEIGHT: f32 = 450.0;
pub const UI_FONT_WEIGHT_STRONG: f32 = 550.0;
pub const UI_FONT_WEIGHT_MEDIUM: f32 = 500.0;
pub const FAMILY_MEDIUM: &str = "desigma-medium";
/// Table headers and sidebar rows of the variables view (ui-spec: weight 600).
pub const UI_FONT_WEIGHT_SEMIBOLD: f32 = 600.0;
pub const FAMILY_SEMIBOLD: &str = "desigma-semibold";
/// Inter's optical-size axis at text sizes.
pub const UI_FONT_OPSZ: f32 = 14.0;

pub fn font_small() -> FontId {
    FontId::new(FONT_SIZE_SMALL, FontFamily::Proportional)
}
pub fn font_small_medium() -> FontId {
    FontId::new(FONT_SIZE_SMALL, FontFamily::Name(FAMILY_MEDIUM.into()))
}
pub fn font_small_strong() -> FontId {
    FontId::new(FONT_SIZE_SMALL, FontFamily::Name(FAMILY_STRONG.into()))
}
pub fn font_body() -> FontId {
    FontId::new(FONT_SIZE_BODY, FontFamily::Proportional)
}
/// Menu rows: Inter at the medium instance.
pub fn font_menu() -> FontId {
    FontId::new(FONT_SIZE_BODY, FontFamily::Name(FAMILY_MEDIUM.into()))
}
pub fn font_body_semibold() -> FontId {
    FontId::new(FONT_SIZE_BODY, FontFamily::Name(FAMILY_SEMIBOLD.into()))
}
pub fn font_body_strong() -> FontId {
    FontId::new(FONT_SIZE_BODY, FontFamily::Name(FAMILY_STRONG.into()))
}
pub fn font_large() -> FontId {
    FontId::new(FONT_SIZE_LARGE, FontFamily::Proportional)
}
pub fn font_large_strong() -> FontId {
    FontId::new(FONT_SIZE_LARGE, FontFamily::Name(FAMILY_STRONG.into()))
}
/// Page titles on the home screen.
pub fn font_title() -> FontId {
    FontId::new(FONT_SIZE_TITLE, FontFamily::Name(FAMILY_STRONG.into()))
}
pub fn font_mono() -> FontId {
    FontId::new(FONT_SIZE_BODY, FontFamily::Monospace)
}

// ---------------------------------------------------------------- radii
pub const RADIUS_SMALL: u8 = 2;
pub const RADIUS_MEDIUM: u8 = 5;
pub const RADIUS_LARGE: u8 = 13;

pub fn cr(r: u8) -> CornerRadius {
    CornerRadius::same(r)
}
pub fn cr_each(nw: u8, ne: u8, se: u8, sw: u8) -> CornerRadius {
    CornerRadius { nw, ne, se, sw }
}

// ---------------------------------------------------------------- spacing

// ---------------------------------------------------------------- shell layout
pub const RAIL_WIDTH: f32 = 56.0;
pub const BORDER_WIDTH: f32 = 1.0;
pub const RAIL_MENU_BUTTON_X: f32 = 12.0;
pub const RAIL_MENU_BUTTON_Y: f32 = 8.0;
pub const RAIL_MENU_BUTTON_SIZE: f32 = 32.0;
pub const RAIL_ITEM_SIZE: f32 = 56.0;
pub const RAIL_FIRST_Y: f32 = 56.0;
pub const RAIL_GROUP_GAP: f32 = 16.0;
pub const RAIL_ICON_SIZE: f32 = 20.0;
/// 1px separators under the logo and before the last group (y relative to the rail top / item cell).
pub const RAIL_SEPARATOR_Y: f32 = 48.0;
pub const RAIL_SEPARATOR_INSET: f32 = 20.0;
pub const RAIL_TILE: f32 = 32.0;
pub const RAIL_ICON_CENTER_Y: f32 = 20.0;
pub const RAIL_LABEL_CENTER_Y: f32 = 44.0;

pub const LEFT_PANEL_WIDTH: f32 = 240.0;
pub const LEFT_HEADER_HEIGHT: f32 = 65.0;
pub const LEFT_FILE_NAME_Y: f32 = 16.0;
pub const LEFT_SUBLABEL_Y: f32 = 40.0;
pub const LEFT_ACTION_Y: f32 = 12.0;
pub const LEFT_SIDEBAR_BTN_RIGHT_PAD: f32 = 12.0;
pub const PAGES_SEARCH_DX: f32 = 28.0;
pub const PANEL_TITLE_ROW_HEIGHT: f32 = 40.0;
pub const PANEL_ACTION_BUTTON: f32 = 24.0;
pub const PANEL_ACTION_RIGHT_PAD: f32 = 8.0;
pub const PANEL_PADDING_X: f32 = 16.0;
pub const PAGE_ROW_HEIGHT: f32 = 24.0;
pub const PAGE_ROW_INSET: f32 = 8.0;
pub const PAGE_ROW_TEXT_PAD: f32 = 8.0;
pub const PAGES_MAX_VISIBLE_ROWS: usize = 5;
pub const PAGE_MENU_WIDTH: f32 = 160.0;
pub const LAYER_HEADER_HEIGHT: f32 = 40.0;
pub const LAYER_ROW_HEIGHT: f32 = 32.0;
pub const LAYER_INDENT: f32 = 16.0;
pub const LAYER_CHEVRON_BOX: f32 = 24.0;
pub const LAYER_CHEVRON_PAD: f32 = 8.0;
pub const LAYER_ICON_SIZE: f32 = 16.0;
pub const LAYER_ICON_GAP: f32 = 0.0;
pub const LAYER_TOGGLE_SIZE: f32 = 24.0;
pub const LAYER_DROP_THICKNESS: f32 = 2.0;

// Find and replace panel and the layers name filter (left sidebar).
pub const FIND_FIELD_H: f32 = 28.0;
pub const FIND_ROW_GAP: f32 = 8.0;
pub const FIND_TOGGLE_SIZE: f32 = 22.0;
pub const FIND_BUTTON_H: f32 = 24.0;
pub const FIND_STATUS_H: f32 = 32.0;
pub const FIND_GROUP_ROW_H: f32 = 28.0;
/// Highlight behind the matched characters of a result or a filtered layer name.
pub const FIND_HIGHLIGHT: Color32 = rgba(0x0D99FF, 0x59);
pub const FIND_HIGHLIGHT_SELECTED: Color32 = rgba(0x7CC4F8, 0x66);
/// Characters of context kept before a match in a result excerpt.
pub const FIND_EXCERPT_LEAD: usize = 12;

pub const RIGHT_PANEL_WIDTH: f32 = 240.0;
/// The present button sits at the right edge, level with the zoom dropdown (8px from the edge).
pub const RIGHT_PRESENT_FROM_RIGHT: f32 = 56.0;
/// Outlined play triangle of the present button.
pub const PRESENT_ICON_W: f32 = 15.0;
pub const PRESENT_ICON_H: f32 = 16.0;
pub const PRESENT_ICON_STROKE: f32 = 1.2;
pub const RIGHT_BTN_Y: f32 = 8.0;
pub const RIGHT_BTN_SIZE: f32 = 32.0;
pub const RIGHT_PRESENT_CHEVRON_W: f32 = 16.0;
pub const RIGHT_TABS_Y: f32 = 48.0;
pub const RIGHT_TABS_H: f32 = 24.0;
pub const RIGHT_TABS_X: f32 = 9.0;
pub const RIGHT_TAB_PAD_X: f32 = 8.0;
pub const RIGHT_ZOOM_FROM_RIGHT: f32 = 63.0;
pub const RIGHT_ZOOM_W: f32 = 55.0;
pub const RIGHT_CONTENT_TOP: f32 = 81.0;
pub const RIGHT_SELECTION_HEADER_Y: f32 = 93.0;
pub const RIGHT_NAME_FIELD_X: f32 = 9.0;
pub const RIGHT_NAME_FIELD_W: f32 = 136.0;
pub const RIGHT_NAME_ACTION_X: [f32; 3] = [153.0, 181.0, 209.0];
pub const SECTION_TITLE_HEIGHT: f32 = 40.0;
pub const SECTION_CONTENT_X: f32 = 17.0;
pub const SECTION_CONTENT_W: f32 = 184.0;
pub const SECTION_TRAILING_X: f32 = 209.0;
pub const FIELD_HEIGHT: f32 = 24.0;
pub const FIELD_WIDTH_HALF: f32 = 88.0;
pub const FIELD_COL_X: [f32; 2] = [17.0, 113.0];
pub const FIELD_PREFIX_W: f32 = 24.0;
/// Prefix labels (X, Y, W, ...) start this far inside the field.
pub const FIELD_PREFIX_X: f32 = 8.0;
/// Prefix icon slot (16px glyph at x+8); the value then starts at x+24.
pub const FIELD_PREFIX_ICON: f32 = 16.0;
/// Glyph box at the slot's left edge (x+8..x+20): Lucide fills its box edge to edge where
/// Figma's art has margins, so the glyph is smaller to leave a gap before the value.
pub const FIELD_PREFIX_GLYPH: f32 = 12.0;
/// Fields without a prefix start their value 8px in.
pub const FIELD_PLAIN_X: f32 = 8.0;
/// Gap between a value and a worded suffix such as " ms".
pub const FIELD_SUFFIX_GAP: f32 = 4.0;
pub const DISTRIBUTE_BTN_X: f32 = 209.0;
pub const SUBLABEL_HEIGHT: f32 = 16.0;
pub const SEGMENT_W: f32 = 29.0;
pub const SEGMENT_GROUP_GAP: f32 = 8.0;
pub const SWATCH_SIZE: f32 = 16.0;
pub const SWATCH_RADIUS: u8 = 2;
pub const SWATCH_ROW_X: f32 = 17.0;
pub const STYLE_ROW_H: f32 = 32.0;
/// Colour-style rows: circle swatch centre (from the row's left) and label x.
pub const STYLE_SWATCH: f32 = 16.0;
pub const STYLE_SWATCH_CX: f32 = 16.0;
pub const STYLE_LABEL_X: f32 = 33.0;
/// Gaps in the page-level Styles section (measured from the Figma capture).
pub const STYLES_LABEL_GAP: f32 = 8.0;
pub const STYLES_GROUP_GAP: f32 = 14.0;
pub const STYLES_BOTTOM_PAD: f32 = 13.0;
/// Gap between a group label and its first row.
pub const STYLES_ROW_GAP: f32 = 9.0;
/// Page background field group: offset below the title, swatch, hex and opacity columns.
pub const PAGE_FIELD_DY: f32 = 4.0;
pub const PAGE_BOTTOM_PAD: f32 = 16.0;
pub const PAGE_SWATCH: f32 = 12.0;
pub const PAGE_SWATCH_CX: f32 = 12.0;
pub const PAGE_HEX_X: f32 = 24.0;
/// Hex text field width in swatch + hex rows (e.g. the Prototype background).
pub const HEX_FIELD_W: f32 = 72.0;
pub const PAGE_DIVIDER_X: f32 = 129.0;
pub const PAGE_OPACITY_PAD: f32 = 8.0;
pub const PAGE_PERCENT_W: f32 = 16.0;
pub const PAGE_PERCENT_RIGHT: f32 = 7.0;
pub const STYLE_FOLDER_ROW_H: f32 = 24.0;
pub const STYLE_INDENT: f32 = 12.0;
pub const OPACITY_FIELD_W: f32 = 48.0;

pub const TOOLBELT_HEIGHT: f32 = 48.0;
pub const TOOLBELT_BOTTOM_MARGIN: f32 = 12.0;
pub const TOOLBELT_PADDING: f32 = 8.0;
pub const TOOL_BUTTON: f32 = 32.0;
pub const TOOL_CHEVRON_W: f32 = 16.0;
pub const TOOL_CHEVRON_GAP: f32 = 0.0;
/// Glyph size of the dropdown chevrons (the 16px box holds a ~6px wide arrow).
pub const TOOL_CHEVRON_ICON: f32 = 11.0;
/// The chevron glyph sits this far right of its button's right edge.
pub const TOOL_CHEVRON_CENTER_DX: f32 = 11.0;
pub const TOOL_GROUP_GAP: f32 = 8.0;
pub const TOOL_ICON_SIZE: f32 = 20.0;
/// Mode switcher: 4 segments (draw, design, prototype, dev) in a padded rounded group.
pub const MODE_SEGMENTS: usize = 5;
pub const MODE_SWITCH_PAD: f32 = 2.0;
pub const MODE_SWITCH_W: f32 = 29.5;
pub const MODE_TILE_RADIUS: u8 = 3;
/// 1px divider before the switcher: distance after the last tool, and before the switcher.
pub const TOOLBAR_DIVIDER_GAP_BEFORE: f32 = 13.0;
pub const TOOLBAR_DIVIDER_GAP_AFTER: f32 = 8.0;

pub const MENU_WIDTH: f32 = 200.0;
pub const MENU_ITEM_HEIGHT: f32 = 24.0;
/// Vertical padding above the first and below the last row.
pub const MENU_PADDING: f32 = 8.0;
pub const MENU_RADIUS: u8 = 13;
/// Highlight pill: inset from the panel edges, corner radius.
pub const MENU_PILL_INSET: f32 = 8.0;
pub const MENU_PILL_RADIUS: u8 = 5;
pub const BG_MENU_SELECTED: Color32 = BG_BRAND;
/// Columns (x from the panel's left edge): check, icon box, label.
pub const MENU_CHECK_X: f32 = 12.0;
pub const MENU_CHECK_BOX: f32 = 16.0;
pub const MENU_ICON_X_CHECKED: f32 = 32.0;
pub const MENU_ICON_BOX: f32 = 24.0;
pub const MENU_LABEL_X_ICON: f32 = 60.0;
pub const MENU_LABEL_X_PLAIN: f32 = 16.0;
/// Label x for rows that have a check column but no icon.
pub const MENU_LABEL_X_CHECK: f32 = 32.0;
/// Icon x when there is no check column; the label follows 4px after the box.
pub const MENU_ICON_X_PLAIN: f32 = 12.0;
pub const MENU_ICON_LABEL_GAP: f32 = 4.0;
pub const MENU_SHORTCUT_PAD: f32 = 16.0;
pub const MENU_CHEVRON_BOX: f32 = 24.0;
pub const MENU_CHEVRON_PAD: f32 = 8.0;
pub const MENU_SEP_MARGIN: f32 = 8.0;
/// Space kept between the label and a right-aligned shortcut / chevron.
pub const MENU_LABEL_GAP: f32 = 16.0;
pub const MENU_SHORTCUT_HIGHLIGHT: Color32 = rgba(0xFFFFFF, 0xCC);
pub const MENU_ICON_SIZE: f32 = 16.0;
pub const MENU_CHEVRON_SIZE: f32 = 12.0;
/// Submenus overlap their parent by this much horizontally.
pub const SUBMENU_OVERLAP: f32 = 0.0;
/// Toolbar dropdown: width, gap right of the tool button, overlap into the toolbar.
pub const TOOL_MENU_WIDTH: f32 = 232.0;
pub const TOOL_MENU_DX: f32 = 2.5;
pub const TOOL_MENU_OVERLAP: f32 = 4.0;
/// Main (logo) menu placement and width.
pub const MAIN_MENU_X: f32 = 12.0;
pub const MAIN_MENU_Y: f32 = 44.0;
pub const MAIN_MENU_WIDTH: f32 = 210.0;
pub const PALETTE_WIDTH: f32 = 320.0;
pub const PALETTE_TOP: f32 = 64.0;
pub const TOAST_RADIUS: u8 = 13;
pub const TOAST_HEIGHT: f32 = 40.0;
pub const TOAST_BOTTOM: f32 = 72.0;
pub const TOAST_SECONDS: f64 = 4.0;
// ---------------------------------------------------------------- comments and version history
/// Comment pin: bubble diameter and the white ring around it.
pub const COMMENT_PIN: f32 = 32.0;
pub const COMMENT_PIN_RING: f32 = 2.0;
pub const COMMENT_PIN_HIT_PAD: f32 = 2.0;
/// Resolved pins are greyed out.
pub const COMMENT_PIN_RESOLVED: Color32 = rgb(0x757575);
/// Avatar colours picked from the author's name.
pub const AVATAR_COLORS: [Color32; 6] = [rgb(0xF24822), rgb(0xFFA629), rgb(0x14AE5C), rgb(0x0C8CE9), rgb(0x9747FF), rgb(0xEE46BC)];
/// Task pins: the palette's amber while in progress, green when done (to do keeps the author colour).
pub const COMMENT_TASK_DOING: Color32 = AVATAR_COLORS[1];
pub const COMMENT_TASK_DONE: Color32 = AVATAR_COLORS[2];
pub const COMMENT_AVATAR: f32 = 24.0;
/// Thread popover: width, gap to its pin, inner padding, header height.
pub const COMMENT_POP_W: f32 = 300.0;
pub const COMMENT_POP_GAP: f32 = 8.0;
pub const COMMENT_PAD: f32 = 12.0;
pub const COMMENT_HEADER_H: f32 = 36.0;
pub const COMMENT_BTN_H: f32 = 24.0;
pub const COMMENT_BOX_MIN_H: f32 = 56.0;
pub const COMMENT_BOX_MAX_H: f32 = 160.0;
pub const COMMENT_ICON_BTN: f32 = 20.0;
/// Comments panel (right side): filter row height and thread card padding.
pub const COMMENT_PANEL_HEADER_H: f32 = 48.0;
pub const COMMENT_FILTER_H: f32 = 24.0;
pub const COMMENT_CARD_PAD: f32 = 12.0;
/// Version history panel rows and the "Viewing" banner.
pub const HISTORY_ROW_H: f32 = 44.0;
pub const HISTORY_DAY_H: f32 = 28.0;
pub const HISTORY_BANNER_H: f32 = 40.0;

// ---------------------------------------------------------------- canvas
pub const CANVAS_OUTLINE_WIDTH: f32 = 1.0;
pub const CANVAS_HOVER_WIDTH: f32 = 1.0;
pub const CANVAS_HANDLE_SIZE: f32 = 7.0;
pub const CANVAS_HANDLE_BORDER: f32 = 1.0;
pub const CANVAS_ROTATE_ZONE: f32 = 12.0;
pub const CANVAS_HANDLE_HIT: f32 = 6.0;
pub const CANVAS_BADGE_OFFSET: f32 = 4.0;
pub const CANVAS_BADGE_PAD_X: f32 = 4.0;
pub const CANVAS_BADGE_HEIGHT: f32 = 16.0;
pub const CANVAS_BADGE_RADIUS: u8 = 2;
pub const CANVAS_LABEL_GAP: f32 = 4.0;
pub const CANVAS_LABEL_MAX_ZOOM_HIDE: f32 = 0.05;
pub const SNAP_THRESHOLD_PX: f32 = 5.0;
pub const GUIDE_WIDTH: f32 = 1.0;
pub const GUIDE_X_SIZE: f32 = 3.0;
pub const GUIDE_TEXT_PAD: f32 = 3.0;
pub const GUIDE_TEXT_HEIGHT: f32 = 14.0;
pub const PEN_POINT_SIZE: f32 = 4.0;
pub const PEN_LINE_WIDTH: f32 = 1.5;
/// Insertion line shown while reordering inside an auto-layout frame.
pub const AL_INDICATOR_WIDTH: f32 = 2.0;
/// Vector edit mode: vertex square, handle dot, hit radii (px).
pub const VEDIT_POINT_SIZE: f32 = 6.0;
pub const VEDIT_HANDLE_RADIUS: f32 = 3.0;
pub const VEDIT_HIT: f32 = 6.0;
pub const VEDIT_SEGMENT_HIT: f32 = 5.0;
pub const DRAG_THRESHOLD_PX: f32 = 3.0;
pub const ZOOM_MIN: f32 = 0.02;
pub const ZOOM_MAX: f32 = 256.0;
pub const FIT_PADDING: f32 = 64.0;
pub const TEXT_CARET_WIDTH: f32 = 1.0;
/// Rulers (Shift+R): 20px strips on the top and left of the canvas.
pub const RULER_SIZE: f32 = 20.0;
pub const RULER_FONT_SIZE: f32 = 9.0;
/// Tick labels: white at 70% (#ffffffb2).
pub const RULER_TEXT: Color32 = rgba(0xFFFFFF, 0xB2);
pub const RULER_TICK: Color32 = rgba(0xFFFFFF, 0x66);
/// Selection extent highlighted on the rulers.
pub const RULER_SELECTION_FILL: Color32 = rgba(0x0D99FF, 0x4D);
/// User guides dragged from the rulers (#f24822).
pub const RULER_GUIDE: Color32 = rgb(0xF24822);
/// Pixel grid lines: #ffffff1a over dark pages, black at the same alpha over light ones.
pub const PIXEL_GRID_LIGHT_LINES: Color32 = rgba(0xFFFFFF, 0x1A);
pub const PIXEL_GRID_DARK_LINES: Color32 = rgba(0x000000, 0x1A);
/// The pixel grid appears from this zoom (800%).
pub const PIXEL_GRID_MIN_ZOOM: f32 = 8.0;
pub fn font_ruler() -> FontId {
    FontId::new(RULER_FONT_SIZE, FontFamily::Proportional)
}
/// Selected text highlight: #0c8ce9 at 35% alpha.
pub const TEXT_SELECTION: Color32 = Color32::from_rgba_premultiplied(4, 49, 81, 89);
/// Find matches on the canvas text: #0c8ce9 at 35% alpha, like a selection.
pub const FIND_CANVAS_MATCH: Color32 = TEXT_SELECTION;
/// Outline of the current find match.
pub const FIND_CANVAS_MATCH_CURRENT: Color32 = BG_BRAND;
/// At most this many matches in view are highlighted per frame.
pub const FIND_CANVAS_MATCH_MAX: usize = 600;
pub const NUDGE: f32 = 1.0;
pub const NUDGE_BIG: f32 = 10.0;

// ---------------------------------------------------------------- dev mode
/// Syntax colours of the Inspect code blocks.
pub const SYNTAX_PLAIN: Color32 = TEXT;
pub const SYNTAX_PROPERTY: Color32 = rgb(0x7CC4F8);
pub const SYNTAX_KEYWORD: Color32 = rgb(0xC792EA);
pub const SYNTAX_STRING: Color32 = rgb(0x9CD67D);
pub const SYNTAX_NUMBER: Color32 = rgb(0xF2A365);
pub const SYNTAX_TYPE: Color32 = rgb(0xFFCB6B);
pub const SYNTAX_FUNCTION: Color32 = rgb(0x82AAFF);
pub const SYNTAX_COMMENT: Color32 = rgb(0x8A8A8A);
pub const SYNTAX_PUNCT: Color32 = rgb(0xB3B3B3);
/// Dev status pills: Ready for dev, Completed, Changed.
pub const DEV_READY: Color32 = rgb(0x14AE5C);
pub const DEV_COMPLETED: Color32 = rgb(0x9747FF);
pub const DEV_CHANGED: Color32 = rgb(0xFFA629);
/// Box model diagram: margin, padding and content tints (as in Figma's Inspect).
pub const BOX_MARGIN: Color32 = rgb(0xF9CB9C);
pub const BOX_PADDING: Color32 = rgb(0xB6D7A8);
pub const BOX_CONTENT: Color32 = rgb(0x9FC5E8);
pub const BOX_TEXT: Color32 = rgb(0x1E1E1E);
/// Annotation callouts on the canvas.
pub const ANNOTATION_BG: Color32 = rgb(0xFFE08A);
pub const ANNOTATION_TEXT: Color32 = rgb(0x1E1E1E);
pub const ANNOTATION_LINE: Color32 = rgb(0xFFB800);
/// Measurement overlay red (distances between layers).
pub const MEASURE: Color32 = rgb(0xF24822);
pub const CODE_LINE_HEIGHT: f32 = 16.0;

// ---------------------------------------------------------------- prototype
/// Noodles, hotspots and flow flags use the brand blue.
pub const PROTO_BLUE: Color32 = BG_BRAND;
pub const PROTO_NOODLE_WIDTH: f32 = 2.0;
pub const PROTO_HOTSPOT_RADIUS: f32 = 6.0;
pub const PROTO_HOTSPOT_RING: f32 = 2.0;
pub const PROTO_HOTSPOT_HIT: f32 = 10.0;
pub const PROTO_FLAG_HEIGHT: f32 = 20.0;
pub const PROTO_FLAG_GAP: f32 = 4.0;
pub const PROTO_ROW_HEIGHT: f32 = 44.0;
pub const PROTO_POPOVER_WIDTH: f32 = 240.0;
pub const PRESENT_BG: Color32 = rgb(0x1E1E1E);
pub const PRESENT_BAR_HEIGHT: f32 = 48.0;
pub const PRESENT_PADDING: f32 = 32.0;
/// Thickness of the plain device bezel drawn around the device screen when presenting.
pub const PRESENT_BEZEL: f32 = 10.0;
pub const PRESENT_MAX_SCALE: f32 = 4.0;

// ---------------------------------------------------------------- app defaults
pub const DEFAULT_WINDOW: (f32, f32) = (1600.0, 1088.0);
pub const MIN_WINDOW: (f32, f32) = (900.0, 600.0);
pub const APP_TITLE: &str = "desigma";
pub const DEFAULT_SHAPE_SIZE: f32 = 100.0;
pub const DEFAULT_TEXT_SIZE: f32 = 16.0;
pub const DEFAULT_PAGE_BG: u32 = 0xF5F5F5;
pub const DEFAULT_STROKE: u32 = 0x000000;
pub const DEFAULT_FRAME_FILL: u32 = 0xFFFFFF;
pub const DEFAULT_SHAPE_FILL: u32 = 0xD9D9D9;
pub const AUTO_LAYOUT_SPACING: f32 = 10.0;
pub const AUTO_LAYOUT_PADDING: f32 = 10.0;
pub const PASTE_OFFSET: f32 = 10.0;

/// Applies the Figma-like dark visuals to egui so any stock widget we do use
/// (text edits, scroll bars, popups) matches the chrome.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG_MENU;
    v.extreme_bg_color = BG_SECONDARY;
    v.faint_bg_color = BG_SECONDARY;
    v.override_text_color = None;
    v.window_stroke = egui::Stroke::new(1.0, BORDER_MENU);
    v.window_corner_radius = cr(RADIUS_MEDIUM);
    v.menu_corner_radius = cr(RADIUS_MEDIUM);
    v.selection.bg_fill = BG_SELECTED;
    v.selection.stroke = egui::Stroke::new(1.0, BORDER_SELECTED);
    v.text_cursor.stroke = egui::Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.bg_fill = BG;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = cr(RADIUS_MEDIUM);
        w.fg_stroke = egui::Stroke::new(1.0, TEXT);
    }
    v.widgets.inactive.bg_fill = BG_SECONDARY;
    v.widgets.inactive.weak_bg_fill = BG_SECONDARY;
    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.active.bg_fill = BG_PRESSED;
    v.widgets.active.weak_bg_fill = BG_PRESSED;
    style.spacing.item_spacing = egui::vec2(0.0, 0.0);
    style.spacing.button_padding = egui::vec2(0.0, 0.0);
    style.spacing.interact_size = egui::vec2(0.0, 0.0);
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.floating = true;
    style.text_styles.insert(egui::TextStyle::Body, font_body());
    style.text_styles.insert(egui::TextStyle::Button, font_body());
    style.text_styles.insert(egui::TextStyle::Small, font_small());
    style.text_styles.insert(egui::TextStyle::Monospace, font_mono());
    ctx.set_global_style(style);
}

/// Installs the bundled Inter variable font as two instances: regular and strong.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let inter = desigma_fonts::bundled().iter().find(|a| a.family == UI_FONT_FAMILY && !a.italic).expect("bundled Inter");
    let instance = |weight: f32| {
        let tweak = egui::FontTweak { coords: egui::epaint::text::VariationCoords::new([("wght", weight), ("opsz", UI_FONT_OPSZ)]), ..Default::default() };
        std::sync::Arc::new(egui::FontData::from_static(inter.data).tweak(tweak))
    };
    fonts.font_data.insert("ui-regular".into(), instance(UI_FONT_WEIGHT));
    fonts.font_data.insert("ui-strong".into(), instance(UI_FONT_WEIGHT_STRONG));
    fonts.font_data.insert("ui-medium".into(), instance(UI_FONT_WEIGHT_MEDIUM));
    fonts.font_data.insert("ui-semibold".into(), instance(UI_FONT_WEIGHT_SEMIBOLD));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui-regular".into());
    let mut strong_family = vec!["ui-strong".to_string()];
    strong_family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name(FAMILY_STRONG.into()), strong_family);
    let mut medium_family = vec!["ui-medium".to_string()];
    medium_family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name(FAMILY_MEDIUM.into()), medium_family);
    let mut semibold_family = vec!["ui-semibold".to_string()];
    semibold_family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name(FAMILY_SEMIBOLD.into()), semibold_family);
    ctx.set_fonts(fonts);
}

/// Perceived luminance test used to choose label colors on the canvas.
pub fn is_light(c: Color32) -> bool {
    let l = (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0;
    l > LIGHT_BG_THRESHOLD
}

// ---------------------------------------------------------------- projects library (home screen)
pub const HOME_SIDEBAR_W: f32 = 264.0;
pub const HOME_PAD: f32 = 32.0;
pub const HOME_HEADER_H: f32 = 64.0;
pub const HOME_CARD_W: f32 = 224.0;
pub const HOME_THUMB_H: f32 = 140.0;
/// Thumbnail plus the three text lines under it.
pub const HOME_CARD_H: f32 = HOME_THUMB_H + 58.0;
pub const HOME_CARD_GAP: f32 = 24.0;
pub const HOME_FOLDER_H: f32 = 48.0;
pub const HOME_LIST_ROW_H: f32 = 44.0;
pub const HOME_THUMB_BG: Color32 = rgb(0x3A3A3A);
// ---------------------------------------------------------------- variables view
// Measured on the Figma UI3 local-variables captures (800x600, dpr 1).
/// Sidebar content width; the 1px border follows it in the full view.
pub const VAR_SIDEBAR_W: f32 = 240.0;
/// Top bar height of the full view (excluding its 1px bottom border).
pub const VAR_BAR_H: f32 = 48.0;
/// Floating panel: size and default top-left.
pub const VAR_FLOAT_W: f32 = 642.0;
pub const VAR_FLOAT_H: f32 = 380.0;
pub const VAR_FLOAT_Y: f32 = 8.0;
pub const VAR_FLOAT_BAR_H: f32 = 40.0;
pub const VAR_HEAD_H: f32 = 40.0;
pub const VAR_ROW_H: f32 = 40.0;
pub const VAR_ROW_PITCH: f32 = 41.0;
/// Name column width (the 1px grid line follows it).
pub const VAR_NAME_COL: f32 = 200.0;
pub const VAR_MODE_MIN: f32 = 140.0;
pub const VAR_MODE_MAX: f32 = 280.0;
pub const VAR_ADD_COL: f32 = 39.0;
pub const VAR_BOTTOM_H: f32 = 41.0;
pub const VAR_SCROLL_H: f32 = 11.0;
pub const VAR_SCROLL_THUMB: Color32 = rgb(0x707070);
pub const VAR_SEARCH_W: f32 = 174.0;
pub const VAR_FILTER_W: f32 = 25.0;
pub const VAR_SIDE_ROW: f32 = 32.0;
pub const VAR_PILL_H: f32 = 24.0;
pub const VAR_ICON_X: f32 = 16.0;
pub const VAR_NAME_X: f32 = 44.0;
pub const VAR_SWATCH: f32 = 16.0;
pub const VAR_HEX_X: f32 = 30.0;
pub const VAR_HEX_W: f32 = 146.0;
pub const VAR_OPACITY_W: f32 = 62.0;
pub const VAR_OPACITY_RIGHT: f32 = 74.0;
pub const VAR_FIELD_H: f32 = 24.0;
pub const VAR_POPOVER_W: f32 = 264.0;

// ---------------------------------------------------------------- board (whiteboard) mode
/// Sticky note and section palette (`0xRRGGBB`), the first is the default: FigJam's eight pastels, then light gray and white.
pub const BOARD_PALETTE: [u32; 10] = [0xFFE58F, 0xFFC48A, 0xFFB3B3, 0xF2B8E6, 0xD5C3FF, 0xB6D4FF, 0xA8E6DD, 0xC4EBA6, 0xE6E6E6, 0xFFFFFF];
/// Ink colours for connectors, markers and text on a board (dark first).
pub const BOARD_INK: [u32; 8] = [0x1E1E1E, 0x757575, 0xF24822, 0xFFA629, 0x14AE5C, 0x0C8CE9, 0x9747FF, 0xEE46BC];
/// Canvas colour of a board and the dots drawn on it.
/// FigJam's drawing palette: black, grey, red, orange, yellow, green, blue, violet, pink.
pub const BOARD_DRAW_INK: [u32; 9] = [0x1E1E1E, 0x757575, 0xF24822, 0xFFA629, 0xFFD43B, 0x14AE5C, 0x0C8CE9, 0x9747FF, 0xEE46BC];
pub const BOARD_BG: u32 = 0xF4F4F4;
pub const BOARD_DOT: Color32 = rgba(0x000000, 0x2E);
/// Distance between canvas dots in page units; the grid doubles while it would be denser than the minimum on screen.
pub const BOARD_DOT_SPACING: f32 = 24.0;
pub const BOARD_DOT_MIN_PX: f32 = 14.0;
/// Magnet dots on a hovered shape: radius, the gap from the shape's side, and the grab radius (screen px).
pub const BOARD_MAGNET_RADIUS: f32 = 5.0;
pub const BOARD_MAGNET_GAP: f32 = 12.0;
pub const BOARD_MAGNET_HIT: f32 = 10.0;
/// New items.
pub const BOARD_SHAPE_W: f32 = 160.0;
pub const BOARD_SHAPE_H: f32 = 120.0;
pub const BOARD_STAMP_SIZE: f32 = 64.0;
pub const BOARD_STICKY_GAP: f32 = 24.0;
pub const BOARD_MARKER_WIDTH: f32 = 6.0;
pub const BOARD_HIGHLIGHTER_WIDTH: f32 = 24.0;
pub const BOARD_HIGHLIGHTER_ALPHA: f32 = 0.4;
pub const BOARD_WASHI_WIDTH: f32 = 24.0;
pub const BOARD_WASHI_ALPHA: f32 = 0.92;
/// Radius of the eraser disc in screen pixels.
pub const BOARD_ERASER_RADIUS: f32 = 10.0;
/// Floating contextual toolbar: height, padding, gap to the selection, swatch size.
pub const BOARD_BAR_H: f32 = 40.0;
pub const BOARD_BAR_PAD: f32 = 6.0;
pub const BOARD_BAR_GAP: f32 = 12.0;
pub const BOARD_BAR_BTN: f32 = 28.0;
pub const BOARD_SWATCH: f32 = 18.0;
pub const BOARD_BAR_SEP: f32 = 8.0;
