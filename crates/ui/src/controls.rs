use crate::theme::Skin;
use gpui_kit::component::{Sizable, button::*};
use gpui_kit::{Styled, prelude::FluentBuilder};

pub(crate) fn toolbar_button(button: Button) -> Button {
    button.cursor_pointer().ghost().small()
}

pub(crate) fn toolbar_toggle(button: Button, active: bool, skin: Skin) -> Button {
    toolbar_button(button).when(active, |button| {
        button.bg(skin.accent.opacity(0.13)).text_color(skin.accent)
    })
}
