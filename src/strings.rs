//! Every user-visible string. English is the base; more locales plug in here later
//! (selected from the TV's `localeInfo.locales.UI`).

pub struct Strings {
    pub favorites: &'static str,
    pub inputs: &'static str,
    pub hidden: &'static str,
    pub press_ok_to_open: &'static str,
    pub hold_ok_for_options: &'static str,
    pub connected: &'static str,
    pub connected_press_ok: &'static str,
    pub no_signal: &'static str,
    pub hidden_app: &'static str,
    pub menu_move: &'static str,
    pub menu_hide: &'static str,
    pub menu_show: &'static str,
    pub menu_cancel: &'static str,
    pub moving: &'static str,
    pub moving_hint: &'static str,
    pub notice_service: &'static str,
    pub notice_app_list: &'static str,
    pub notice_launch_failed: &'static str,
    pub notice_preview_launch: &'static str,
    pub days: [&'static str; 7],
    pub months: [&'static str; 12],
}

pub static EN: Strings = Strings {
    favorites: "Favorites",
    inputs: "Inputs",
    hidden: "Hidden apps",
    press_ok_to_open: "Press OK to open",
    hold_ok_for_options: "Hold OK for options",
    connected: "Connected",
    connected_press_ok: "Connected · Press OK to switch",
    no_signal: "No signal",
    hidden_app: "Hidden from Favorites · Hold OK to show",
    menu_move: "Move",
    menu_hide: "Hide",
    menu_show: "Show in Favorites",
    menu_cancel: "Cancel",
    moving: "Moving",
    moving_hint: "Left / Right to move · OK to finish",
    notice_service: "Cannot reach the TV service",
    notice_app_list: "App list unavailable",
    notice_launch_failed: "Could not launch the app",
    notice_preview_launch: "Preview: would launch",
    days: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    months: [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
    ],
};

pub fn current() -> &'static Strings {
    &EN
}

/// Every character the UI may draw from these strings (for glyph prewarming).
pub fn charset(strings: &Strings) -> String {
    let mut all = String::new();
    for text in [
        strings.favorites,
        strings.inputs,
        strings.hidden,
        strings.press_ok_to_open,
        strings.hold_ok_for_options,
        strings.connected,
        strings.connected_press_ok,
        strings.no_signal,
        strings.hidden_app,
        strings.menu_move,
        strings.menu_hide,
        strings.menu_show,
        strings.menu_cancel,
        strings.moving,
        strings.moving_hint,
    ] {
        all.push_str(text);
        all.push_str(&text.to_uppercase());
    }
    for text in strings.days.iter().chain(strings.months.iter()) {
        all.push_str(text);
    }
    all
}
