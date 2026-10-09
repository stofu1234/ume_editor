// Derived from gpui-component 0.7.1, src/menu/app_menu_bar.rs
// (https://github.com/longbridge/gpui-kit), licensed under the Apache License 2.0.
// Copyright (c) Longbridge and gpui-component contributors.
//
// Modified for the ume_editor PoC: added Windows-style keyboard access
// (Alt alone highlights the bar, Alt+<mnemonic> opens a menu, arrow keys,
// Enter/Down to open, Esc/Alt to cancel), built popups with the public
// PopupMenu builder, and removed the tests.

use gpui::{
    App, AppContext as _, ClickEvent, Context, DismissEvent, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton, OwnedMenu,
    OwnedMenuItem, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, anchored, deferred, div, prelude::FluentBuilder, px,
};
use gpui_base::actions::{Cancel, SelectLeft, SelectRight};
use gpui_component::{
    Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::PopupMenu,
};

const CONTEXT: &str = "UmeMenuBar";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(CONTEXT)),
    ]);
}

/// Returns the mnemonic in a label such as "ファイル(F)".
fn mnemonic(label: &str) -> Option<char> {
    let start = label.rfind('(')?;
    let mut chars = label[start + 1..].chars();
    let ch = chars.next()?;
    (chars.next() == Some(')')).then(|| ch.to_ascii_lowercase())
}

pub struct MenuBar {
    menus: Vec<Entity<MenuBarItem>>,
    /// The menu whose popup is open.
    selected_index: Option<usize>,
    /// The menu highlighted from the keyboard while no popup is open.
    armed_index: Option<usize>,
    /// Where actions go and where focus returns when the menu closes.
    action_context: Option<FocusHandle>,
    focus_handle: FocusHandle,
}

impl MenuBar {
    pub fn new(menus: Vec<OwnedMenu>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let menu_bar = cx.entity();
            let menus = menus
                .iter()
                .enumerate()
                .map(|(ix, menu)| MenuBarItem::new(ix, menu, menu_bar.clone(), cx))
                .collect();
            Self {
                menus,
                selected_index: None,
                armed_index: None,
                action_context: None,
                focus_handle: cx.focus_handle(),
            }
        })
    }

    pub fn is_active(&self) -> bool {
        self.selected_index.is_some() || self.armed_index.is_some()
    }

    /// Alt pressed and released alone: highlight the bar, or leave it if active.
    pub fn toggle_armed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_active() {
            self.close(window, cx);
        } else {
            self.action_context = window.focused(cx);
            self.armed_index = Some(0);
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    /// Opens the menu whose mnemonic is `key`. Returns false if none matches.
    pub fn open_by_mnemonic(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ch) = key.chars().next().filter(|_| key.chars().count() == 1) else {
            return false;
        };
        let ch = ch.to_ascii_lowercase();
        let ix = self
            .menus
            .iter()
            .position(|m| mnemonic(&m.read(cx).name) == Some(ch));
        match ix {
            Some(ix) => {
                self.set_selected_index(Some(ix), window, cx);
                true
            }
            None => false,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.armed_index = None;
        self.set_selected_index(None, window, cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(armed) = self.armed_index else {
            return;
        };
        let key = event.keystroke.key.as_str();
        match key {
            "down" | "enter" | "space" => self.set_selected_index(Some(armed), window, cx),
            _ if !event.keystroke.modifiers.modified() => {
                if !self.open_by_mnemonic(key, window, cx) {
                    return;
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn on_move_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn on_move_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.menus.len() as isize;
        if let Some(ix) = self.selected_index {
            let new_ix = (ix as isize + delta).rem_euclid(len) as usize;
            self.set_selected_index(Some(new_ix), window, cx);
        } else if let Some(ix) = self.armed_index {
            self.armed_index = Some((ix as isize + delta).rem_euclid(len) as usize);
            cx.notify();
        }
    }

    fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn set_selected_index(
        &mut self,
        ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix.is_some() {
            if self.action_context.is_none() {
                self.action_context = window.focused(cx);
            }
            self.armed_index = None;
        } else {
            if let Some(action_context) = self.action_context.take() {
                action_context.focus(window, cx);
            }
            self.armed_index = None;
        }
        self.selected_index = ix;
        cx.notify();
    }

    fn has_activated_menu(&self) -> bool {
        self.selected_index.is_some()
    }
}

impl Focusable for MenuBar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Clicking elsewhere moves focus away; drop the keyboard highlight then.
        if self.armed_index.is_some() && !self.focus_handle.is_focused(window) {
            self.armed_index = None;
            self.action_context = None;
        }
        h_flex()
            .id("ume-menu-bar")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_cancel))
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .gap_x_1()
            .children(self.menus.clone())
    }
}

/// A menu in the menu bar.
pub struct MenuBarItem {
    menu_bar: Entity<MenuBar>,
    ix: usize,
    name: SharedString,
    menu: OwnedMenu,
    popup_menu: Option<Entity<PopupMenu>>,
    _subscription: Option<Subscription>,
}

impl MenuBarItem {
    fn new(ix: usize, menu: &OwnedMenu, menu_bar: Entity<MenuBar>, cx: &mut App) -> Entity<Self> {
        let name = menu.name.clone();
        cx.new(|_| Self {
            ix,
            menu_bar,
            name,
            menu: menu.clone(),
            popup_menu: None,
            _subscription: None,
        })
    }

    fn is_open(&self, cx: &App) -> bool {
        self.menu_bar.read(cx).selected_index == Some(self.ix)
    }

    fn is_armed(&self, cx: &App) -> bool {
        self.menu_bar.read(cx).armed_index == Some(self.ix)
    }

    fn build_popup_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<PopupMenu> {
        // The popup's action context can only be set while building it, so the popup
        // is built each time the menu opens and dropped when it is dismissed.
        let popup_menu = match self.popup_menu.as_ref() {
            Some(menu) => menu.clone(),
            None => {
                let action_context = self.menu_bar.read(cx).action_context.clone();
                let items = self.menu.items.clone();
                let popup_menu = PopupMenu::build(window, cx, move |menu, _, _| {
                    let mut menu = match action_context {
                        Some(handle) => menu.action_context(handle),
                        None => menu,
                    };
                    for item in items {
                        menu = match item {
                            OwnedMenuItem::Action {
                                name,
                                action,
                                disabled,
                                ..
                            } => menu.menu_with_disabled(name, action, disabled),
                            OwnedMenuItem::Separator => menu.separator(),
                            // Submenus and OS menus are not needed for the PoC.
                            _ => menu,
                        };
                    }
                    menu
                });
                self._subscription =
                    Some(cx.subscribe_in(&popup_menu, window, Self::handle_dismiss));
                self.popup_menu = Some(popup_menu.clone());
                popup_menu
            }
        };

        let focus_handle = popup_menu.read(cx).focus_handle(cx);
        if !focus_handle.contains_focused(window, cx) {
            focus_handle.focus(window, cx);
        }
        popup_menu
    }

    fn handle_dismiss(
        &mut self,
        _: &Entity<PopupMenu>,
        _: &DismissEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._subscription.take();
        self.popup_menu.take();
        self.menu_bar
            .update(cx, |state, cx| state.on_cancel(&Cancel, window, cx));
    }

    fn handle_trigger_click(
        &mut self,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, ClickEvent::Mouse(_)) {
            return;
        }
        self.toggle(window, cx);
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_open = self.is_open(cx);
        self.menu_bar.update(cx, |state, cx| {
            let new_ix = if is_open { None } else { Some(self.ix) };
            state.set_selected_index(new_ix, window, cx);
        });
    }

    fn handle_hover(&mut self, hovered: &bool, window: &mut Window, cx: &mut Context<Self>) {
        if !*hovered || !self.menu_bar.read(cx).has_activated_menu() {
            return;
        }
        self.menu_bar.update(cx, |state, cx| {
            state.set_selected_index(Some(self.ix), window, cx)
        });
    }
}

impl Render for MenuBarItem {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_open = self.is_open(cx);
        let is_armed = self.is_armed(cx);

        div()
            .id(self.ix)
            .relative()
            .child(
                Button::new("menu")
                    .small()
                    .py_0p5()
                    .compact()
                    .ghost()
                    .label(self.name.clone())
                    .selected(is_armed)
                    .open(is_open)
                    .on_mouse_down(
                        MouseButton::Left,
                        window.listener_for(&cx.entity(), move |this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle(window, cx);
                        }),
                    )
                    .on_click(cx.listener(Self::handle_trigger_click)),
            )
            .on_hover(cx.listener(Self::handle_hover))
            .when(is_open, |this| {
                this.child(deferred(
                    anchored()
                        .anchor(gpui::Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .size_full()
                                .occlude()
                                .top_1()
                                .child(self.build_popup_menu(window, cx)),
                        ),
                ))
            })
    }
}
