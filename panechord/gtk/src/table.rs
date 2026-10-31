use super::*;
use panechord_runtime::{Table, TableColumn, TableRow, TextOverflow};
use std::sync::Arc;

struct TableState {
    data: RefCell<Table>,
    rows: gio::ListStore,
    headers: gtk::Box,
    separator: gtk::Separator,
    empty: gtk::Label,
    groups: RefCell<Vec<gtk::SizeGroup>>,
}

pub(super) fn apply_text_style(label: &gtk::Label, text: Option<panechord_runtime::TextStyle>) {
    label.remove_css_class("heading");
    label.remove_css_class("monospace");
    let Some(text) = text else {
        label.set_attributes(None);
        return;
    };
    let attributes = gtk::pango::AttrList::new();
    match text.role {
        panechord_runtime::TextRole::Body => {}
        panechord_runtime::TextRole::Heading => label.add_css_class("heading"),
        panechord_runtime::TextRole::Code => {
            label.add_css_class("monospace");
            attributes.insert(gtk::pango::AttrString::new_family("monospace"));
        }
    }
    attributes.insert(gtk::pango::AttrFloat::new_scale(f64::from(text.scale)));
    if let Some([red, green, blue]) = text.foreground {
        attributes.insert(gtk::pango::AttrColor::new_foreground(
            u16::from(red) * 257,
            u16::from(green) * 257,
            u16::from(blue) * 257,
        ));
    }
    label.set_attributes(Some(&attributes));
}

fn label(column: &TableColumn, value: &str, heading: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    label.set_hexpand(column.stretch);
    label.set_halign(gtk::Align::Fill);
    label.set_valign(gtk::Align::Start);
    if let Some(width) = column.width {
        label.set_size_request(width.round() as i32, -1);
    }
    apply_text_style(
        &label,
        Some(if heading {
            panechord_runtime::TextStyle {
                role: panechord_runtime::TextRole::Heading,
                ..Default::default()
            }
        } else {
            column.text
        }),
    );
    match column.overflow {
        TextOverflow::Wrap if !heading => {
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_max_width_chars(1);
        }
        TextOverflow::EllipsizeMiddle if !heading => {
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle)
        }
        _ => label.set_ellipsize(gtk::pango::EllipsizeMode::End),
    }
    label
}

impl TableState {
    fn replace(&self, data: Table) {
        let previous = self.data.replace(data.clone());
        self.empty.set_text(&data.empty_text);
        self.empty.set_visible(data.rows.is_empty());
        let headers = data.columns.iter().any(|column| !column.title.is_empty());
        self.headers.set_visible(headers);
        self.separator.set_visible(headers);
        let columns_changed =
            previous.columns != data.columns || previous.column_gap != data.column_gap;
        if columns_changed {
            while let Some(child) = self.headers.first_child() {
                self.headers.remove(&child);
            }
            self.headers.set_spacing(data.column_gap.round() as i32);
            let groups: Vec<_> = data
                .columns
                .iter()
                .map(|column| {
                    let group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
                    let header = label(column, &column.title, true);
                    group.add_widget(&header);
                    self.headers.append(&header);
                    group
                })
                .collect();
            self.groups.replace(groups);
        }
        if columns_changed || previous.rows != data.rows {
            let mut header = self.headers.first_child();
            for (index, column) in data.columns.iter().enumerate() {
                let widget = header.take().unwrap();
                header = widget.next_sibling();
                if column.width.is_none() && !column.stretch {
                    let sample = label(column, "", false);
                    let layout = sample.layout();
                    layout.set_width(-1);
                    layout.set_ellipsize(gtk::pango::EllipsizeMode::None);
                    let width = data.rows.iter().fold(0, |width, row| {
                        layout.set_text(&row.cells[index].text);
                        width.max(layout.pixel_size().0)
                    });
                    widget.set_size_request(width, -1);
                }
            }
        }
        let mut prefix = 0;
        let mut suffix = 0;
        if !columns_changed && previous.row_gap == data.row_gap {
            while prefix < previous.rows.len().min(data.rows.len())
                && previous.rows[prefix] == data.rows[prefix]
            {
                prefix += 1;
            }
            while suffix < previous.rows.len().min(data.rows.len()) - prefix
                && previous.rows[previous.rows.len() - 1 - suffix]
                    == data.rows[data.rows.len() - 1 - suffix]
            {
                suffix += 1;
            }
        }
        let objects: Vec<_> = data.rows[prefix..data.rows.len() - suffix]
            .iter()
            .map(|row| glib::BoxedAnyObject::new(row.clone()))
            .collect();
        self.rows.splice(
            prefix as u32,
            (previous.rows.len() - prefix - suffix) as u32,
            &objects,
        );
    }
}

impl View {
    pub fn table(&self, parent: &Parent, id: &str, data: Binding<Table>) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let headers = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        root.append(&headers);
        root.append(&separator);
        let state = Rc::new(TableState {
            data: RefCell::new(Table::default()),
            rows: gio::ListStore::new::<glib::BoxedAnyObject>(),
            headers,
            separator,
            empty: gtk::Label::new(None),
            groups: RefCell::new(Vec::new()),
        });
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let item = object.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
        });
        let weak = Rc::downgrade(&state);
        let row_css = gtk::CssProvider::new();
        row_css.load_from_data("row { padding: 0; }");
        factory.connect_bind(move |_, object| {
            let Some(state) = weak.upgrade() else { return };
            let item = object.downcast_ref::<gtk::ListItem>().unwrap();
            let row = item
                .item()
                .unwrap()
                .downcast::<glib::BoxedAnyObject>()
                .unwrap();
            let row = row.borrow::<Arc<TableRow>>();
            let body = item.child().unwrap().downcast::<gtk::Box>().unwrap();
            if let Some(row) = body.parent() {
                row.style_context()
                    .add_provider(&row_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }
            let data = state.data.borrow();
            body.set_spacing(data.column_gap.round() as i32);
            body.set_margin_bottom(data.row_gap.round() as i32);
            body.set_widget_name(&row.key);
            for (index, (column, cell)) in data.columns.iter().zip(&row.cells).enumerate() {
                let mut column = column.clone();
                column.text.foreground = cell.foreground.or(column.text.foreground);
                let label = label(&column, &cell.text, false);
                label.set_selectable(true);
                state.groups.borrow()[index].add_widget(&label);
                body.append(&label);
            }
        });
        factory.connect_unbind(|_, object| {
            let item = object.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(body) = item
                .child()
                .and_then(|child| child.downcast::<gtk::Box>().ok())
            {
                while let Some(child) = body.first_child() {
                    body.remove(&child);
                }
            }
        });
        let selection = gtk::NoSelection::new(Some(state.rows.clone()));
        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.set_vexpand(true);
        list.set_hexpand(true);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_vexpand(true);
        scroll.set_hexpand(true);
        scroll.set_child(Some(&list));
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroll));
        state.empty.set_halign(gtk::Align::Center);
        state.empty.set_valign(gtk::Align::Center);
        overlay.add_overlay(&state.empty);
        overlay.set_vexpand(true);
        root.append(&overlay);
        root.set_vexpand(true);
        self.mount_native(parent, id, &root);
        let weak = Rc::downgrade(&state);
        let owner = self.downgrade();
        self.bind_native(id, data, &root, move |_, data| {
            if let Some(state) = weak.upgrade() {
                match data.validate() {
                    Ok(()) => state.replace(data),
                    Err(error) => {
                        if let Some(view) = owner.upgrade() {
                            view.report_error(error.into());
                        }
                    }
                }
            }
        });
        self.keep_at(id, Subscription::new(move || drop(state)));
    }
}
