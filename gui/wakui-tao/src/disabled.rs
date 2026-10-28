//! iced 0.14 pick lists/sliders require callbacks. This wrapper deliberately
//! exposes no update/operate/overlay path, including for previously open menus.
use iced::{
    Length, Size,
    advanced::{Layout, Widget, layout, mouse, renderer, widget::Tree},
};

use crate::binding::{El, Message};
type Renderer = iced_tiny_skia::Renderer;

pub fn gate(element: El<'static>, enabled: bool) -> El<'static> {
    if enabled {
        element
    } else {
        iced::Element::new(Disabled(element))
    }
}

struct Disabled(El<'static>);

impl Widget<Message, iced::Theme, Renderer> for Disabled {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.0)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.0));
    }

    fn size(&self) -> Size<Length> {
        self.0.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.0
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.0.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            mouse::Cursor::Unavailable,
            viewport,
        );
    }
}
