//! Geometry and an iced advanced container. No layout-kind fallback.
use super::{Element, Message};
use iced::{advanced::{layout, mouse, overlay, renderer, widget::{self, Tree}, Clipboard, Shell, Widget},
    Event, Length, Point, Rectangle, Size, Vector};
use wakui2_runtime::{Layout as Policy, Style};
type Renderer = iced_tiny_skia::Renderer;

pub(crate) fn finite(v: f32) -> f32 { if v.is_finite() { v.max(0.0) } else { 0.0 } }
fn coordinate(v: f32) -> f32 { if v.is_finite() { v } else { 0.0 } }

/// Geometry is in logical pixels; child sizes include padding, not margins.
pub fn geometry(policy: Policy, available: f32, children: &[(Size, Style)]) -> (Size, Vec<Point>) {
    let available = if available.is_finite() { available.max(0.0) } else { 4096.0 };
    let mut positions = vec![Point::ORIGIN; children.len()];
    let mut extent = Size::ZERO;
    match policy {
        Policy::Stack { gap, .. } | Policy::Flex { gap } => {
            let horizontal = matches!(policy, Policy::Flex { .. }) || horizontal_policy(policy);
            let wrap = matches!(policy, Policy::Flex { .. });
            let gap = finite(gap);
            let (mut x, mut y, mut line) = (0.0f32, 0.0f32, 0.0f32);
            for (index, (size, style)) in children.iter().enumerate() {
                let m = style.margin;
                let width = finite(size.width) + finite(m.left) + finite(m.right);
                let height = finite(size.height) + finite(m.top) + finite(m.bottom);
                if wrap && x > 0.0 && x + width > available { x = 0.0; y += line + gap; line = 0.0; }
                positions[index] = Point::new(x + finite(m.left), y + finite(m.top));
                extent.width = extent.width.max(x + width);
                extent.height = extent.height.max(y + height);
                if horizontal { x += width + gap; line = line.max(height); }
                else { y += height + gap; }
            }
        }
        Policy::Absolute => {
            for (index, (size, style)) in children.iter().enumerate() {
                let p = style.placement; let m = style.margin;
                let x = coordinate(p.x) + finite(m.left);
                let y = coordinate(p.y) + finite(m.top);
                positions[index] = Point::new(x, y);
                extent.width = extent.width.max(x + size.width + finite(m.right));
                extent.height = extent.height.max(y + size.height + finite(m.bottom));
            }
        }
        Policy::Grid { columns, gap } => {
            let cols = columns.clamp(1, 4096) as usize;
            let gap = finite(gap);
            let cell = ((available - gap * (cols - 1) as f32) / cols as f32).max(0.0);
            let rows = children.iter().map(|(_, s)| grid_cell(s, cols).0 + grid_cell(s, cols).2).max().unwrap_or(0);
            let mut heights = vec![0.0f32; rows];
            for (size, style) in children {
                let (r, _, span, _) = grid_cell(style, cols);
                let need = finite(size.height) + finite(style.margin.top) + finite(style.margin.bottom);
                let occupied: f32 = heights[r..r + span].iter().sum::<f32>() + gap * (span - 1) as f32;
                if need > occupied {
                    for h in &mut heights[r..r + span] { *h += (need - occupied) / span as f32; }
                }
            }
            for (index, (_, style)) in children.iter().enumerate() {
                let (r, c, _, _) = grid_cell(style, cols);
                positions[index] = Point::new(c as f32 * (cell + gap) + finite(style.margin.left),
                    heights[..r].iter().sum::<f32>() + r as f32 * gap + finite(style.margin.top));
            }
            extent = Size::new(available, heights.iter().sum::<f32>() + gap * rows.saturating_sub(1) as f32);
        }
    }
    (extent, positions)
}
fn horizontal_policy(policy: Policy) -> bool { matches!(policy, Policy::Stack { horizontal: true, .. }) }
fn grid_cell(style: &Style, cols: usize) -> (usize, usize, usize, usize) {
    let p = style.placement;
    let row = p.row.clamp(0, 4095) as usize;
    let col = (p.column.max(0) as usize).min(cols - 1);
    (row, col, p.row_span.clamp(1, 4096) as usize, (p.column_span.max(1) as usize).min(cols - col))
}

pub(crate) struct Panel { policy: Policy, keys: Vec<String>, styles: Vec<Style>, children: Vec<Element> }
impl Panel {
    pub fn element(policy: Policy, children: Vec<(String, Style, Element)>) -> Element {
        let mut panel = Self { policy, keys: Vec::new(), styles: Vec::new(), children: Vec::new() };
        for (key, style, element) in children { panel.keys.push(key); panel.styles.push(style); panel.children.push(element); }
        Element::new(panel)
    }
}
#[derive(Default)]
struct Keys(Vec<String>);
impl Widget<Message, iced::Theme, Renderer> for Panel {
    fn size(&self) -> Size<Length> { Size::new(Length::Shrink, Length::Shrink) }
    fn tag(&self) -> widget::tree::Tag { widget::tree::Tag::of::<Keys>() }
    fn state(&self) -> widget::tree::State { widget::tree::State::new(Keys(self.keys.clone())) }
    fn children(&self) -> Vec<Tree> { self.children.iter().map(Tree::new).collect() }
    fn diff(&self, tree: &mut Tree) {
        let old_keys = std::mem::replace(&mut tree.state.downcast_mut::<Keys>().0, self.keys.clone());
        let mut old: std::collections::HashMap<_, _> = old_keys.into_iter().zip(std::mem::take(&mut tree.children)).collect();
        tree.children = self.keys.iter().zip(&self.children).map(|(key, child)| {
            if let Some(mut state) = old.remove(key) { state.diff(child.as_widget()); state }
            else { Tree::new(child) }
        }).collect();
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        let available = limits.max().width;
        let mut nodes = Vec::new();
        for ((child, state), style) in self.children.iter_mut().zip(&mut tree.children).zip(&self.styles) {
            let mut width = available;
            if let Policy::Grid { columns, gap } = self.policy {
                let cols = columns.clamp(1, 4096) as usize;
                let (_, _, _, span) = grid_cell(style, cols);
                width = ((available - finite(gap) * (cols - 1) as f32) / cols as f32).max(0.0) * span as f32
                    + finite(gap) * (span - 1) as f32;
            }
            width = (width - finite(style.margin.left) - finite(style.margin.right)).max(0.0);
            let max = Size::new(width, (limits.max().height - finite(style.margin.top) - finite(style.margin.bottom)).max(0.0));
            let mut child_limits = layout::Limits::new(Size::ZERO, max);
            if let Some(w) = style.placement.width { child_limits = child_limits.width(Length::Fixed(finite(w))); }
            if let Some(h) = style.placement.height { child_limits = child_limits.height(Length::Fixed(finite(h))); }
            nodes.push(child.as_widget_mut().layout(state, renderer, &child_limits));
        }
        let measured: Vec<_> = nodes.iter().zip(&self.styles).map(|(n, s)| (n.size(), *s)).collect();
        let (size, positions) = geometry(self.policy, available, &measured);
        let nodes = nodes.into_iter().zip(positions).map(|(n, p)| n.move_to(p)).collect();
        layout::Node::with_children(limits.resolve(Length::Shrink, Length::Shrink, size), nodes)
    }
    fn draw(&self, tree: &Tree, renderer: &mut Renderer, theme: &iced::Theme, style: &renderer::Style,
        layout: layout::Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        for ((child, state), bounds) in self.children.iter().zip(&tree.children).zip(layout.children()) {
            child.as_widget().draw(state, renderer, theme, style, bounds, cursor, viewport);
        }
    }
    fn update(&mut self, tree: &mut Tree, event: &Event, layout: layout::Layout<'_>, cursor: mouse::Cursor,
        renderer: &Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        // Reverse hit order agrees with draw order for overlapping absolute/grid children.
        let layouts: Vec<_> = layout.children().collect();
        for ((child, state), bounds) in self.children.iter_mut().zip(&mut tree.children).zip(layouts).rev() {
            child.as_widget_mut().update(state, event, bounds, cursor, renderer, clipboard, shell, viewport);
        }
    }
    fn operate(&mut self, tree: &mut Tree, layout: layout::Layout<'_>, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, state), bounds) in self.children.iter_mut().zip(&mut tree.children).zip(layout.children()) {
                child.as_widget_mut().operate(state, bounds, renderer, operation);
            }
        });
    }
    fn mouse_interaction(&self, tree: &Tree, layout: layout::Layout<'_>, cursor: mouse::Cursor,
        viewport: &Rectangle, renderer: &Renderer) -> mouse::Interaction {
        self.children.iter().zip(&tree.children).zip(layout.children()).map(|((c, s), l)|
            c.as_widget().mouse_interaction(s, l, cursor, viewport, renderer)).max().unwrap_or_default()
    }
    fn overlay<'a>(&'a mut self, tree: &'a mut Tree, layout: layout::Layout<'a>, renderer: &Renderer,
        viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'a, Message, iced::Theme, Renderer>> {
        overlay::from_children(&mut self.children, tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wakui2_runtime::Edges;
    #[test]
    fn stack_and_flex_have_real_distinct_geometry() {
        let s = Style::default(); let children = vec![(Size::new(60.0, 20.0), s); 3];
        let (size, p) = geometry(Policy::Flex { gap: 5.0 }, 130.0, &children);
        assert_eq!(p, vec![Point::ORIGIN, Point::new(65.0, 0.0), Point::new(0.0, 25.0)]);
        assert_eq!(size, Size::new(125.0, 45.0));
        let (_, p) = geometry(Policy::Stack { horizontal: true, gap: 5.0 }, 130.0, &children);
        assert_eq!(p[2], Point::new(130.0, 0.0));
    }
    #[test]
    fn grid_placement_spans_and_absolute_margins() {
        let mut a = Style::default(); a.placement.column_span = 2; a.placement.row_span = 2;
        let mut b = Style::default(); b.placement.row = 2; b.placement.column = 1;
        let (size, p) = geometry(Policy::Grid { columns: 2, gap: 10.0 }, 210.0,
            &[(Size::new(210.0, 50.0), a), (Size::new(100.0, 20.0), b)]);
        assert_eq!(p[1], Point::new(110.0, 60.0)); assert_eq!(size.height, 80.0);
        b.margin = Edges { left: 3.0, top: 4.0, right: 2.0, bottom: 1.0 };
        b.placement.x = 11.0; b.placement.y = 12.0;
        let (size, p) = geometry(Policy::Absolute, 100.0, &[(Size::new(20.0, 10.0), b)]);
        assert_eq!(p[0], Point::new(14.0, 16.0)); assert_eq!(size, Size::new(36.0, 27.0));
    }

    struct Probe(u32);
    impl Widget<Message, iced::Theme, Renderer> for Probe {
        fn size(&self) -> Size<Length> { Size::new(Length::Shrink, Length::Shrink) }
        fn tag(&self) -> widget::tree::Tag { widget::tree::Tag::of::<u32>() }
        fn state(&self) -> widget::tree::State { widget::tree::State::new(self.0) }
        fn layout(&mut self, _: &mut Tree, _: &Renderer, _: &layout::Limits) -> layout::Node { layout::Node::new(Size::ZERO) }
        fn draw(&self, _: &Tree, _: &mut Renderer, _: &iced::Theme, _: &renderer::Style,
            _: layout::Layout<'_>, _: mouse::Cursor, _: &Rectangle) {}
    }
    #[test]
    fn iced_state_follows_mount_keys_not_sibling_indices() {
        let panel = |values: &[(&str, u32)]| Panel::element(Policy::Stack { horizontal: false, gap: 0.0 },
            values.iter().map(|(key, value)| (key.to_string(), Style::default(), Element::new(Probe(*value)))).collect());
        let first = panel(&[("a#1", 1), ("b#2", 2)]);
        let mut tree = Tree::new(first.as_widget());
        *tree.children[1].state.downcast_mut::<u32>() = 42;
        let reordered = panel(&[("b#2", 200), ("a#1", 100)]); tree.diff(reordered.as_widget());
        assert_eq!(*tree.children[0].state.downcast_ref::<u32>(), 42);
        assert_eq!(*tree.children[1].state.downcast_ref::<u32>(), 1);
        let replaced = panel(&[("b#3", 3)]); tree.diff(replaced.as_widget());
        assert_eq!(*tree.children[0].state.downcast_ref::<u32>(), 3);
    }
    #[test]
    fn actual_iced_nodes_include_margins_padding_and_fixed_placement() {
        let view = crate::View::new(wakui2_runtime::Binding::constant("geometry".into()));
        view.mount(&view.root(), "leaf", std::rc::Rc::new(|| iced::widget::Space::new().width(10).height(8).into()), Vec::new());
        let mut style = Style::default();
        style.margin = Edges { left: 3.0, right: 2.0, top: 4.0, bottom: 1.0 };
        style.padding = Edges { left: 5.0, right: 7.0, top: 2.0, bottom: 3.0 };
        view.style("leaf", style);
        let renderer = Renderer::new(iced::Font::default(), iced::Pixels(16.0));
        let limits = layout::Limits::new(Size::ZERO, Size::new(300.0, 300.0));
        let mut element = view.root().0.element(); let mut tree = Tree::new(element.as_widget());
        let node = element.as_widget_mut().layout(&mut tree, &renderer, &limits);
        let child = &node.children()[0].children()[0];
        assert_eq!(child.bounds(), Rectangle { x: 3.0, y: 4.0, width: 22.0, height: 13.0 });
        style.placement.width = Some(40.0); style.placement.height = Some(30.0); view.style("leaf", style);
        let mut element = view.root().0.element(); tree.diff(element.as_widget());
        let node = element.as_widget_mut().layout(&mut tree, &renderer, &limits);
        assert_eq!(node.children()[0].children()[0].size(), Size::new(40.0, 30.0));
    }
}