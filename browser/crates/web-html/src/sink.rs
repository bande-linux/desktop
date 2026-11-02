use web_atoms::{Atom, QualName};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum QuirksMode {
    #[default]
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Attr {
    pub(crate) name: QualName,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

#[derive(Clone, Copy)]
pub struct Attributes<'a> {
    pub(crate) list: &'a [Attr],
    pub(crate) values: &'a str,
}

impl<'a> Attributes<'a> {
    pub fn iter(self) -> impl ExactSizeIterator<Item = (QualName, &'a str)> + 'a {
        let values = self.values;
        self.list.iter().map(move |attr| {
            let value = values
                .get(attr.start as usize..attr.end as usize)
                .unwrap_or_default();
            (attr.name, value)
        })
    }

    pub fn get(self, name: QualName) -> Option<&'a str> {
        self.iter().find(|&(n, _)| n == name).map(|(_, v)| v)
    }

    pub fn len(self) -> usize {
        self.list.len()
    }

    pub fn is_empty(self) -> bool {
        self.list.is_empty()
    }
}

pub trait TreeSink {
    type Node: Copy + Eq;

    fn intern(&mut self, name: &str) -> Atom;
    fn document(&self) -> Self::Node;
    fn create_element(&mut self, name: QualName, attributes: Attributes<'_>) -> Self::Node;
    fn create_comment(&mut self, text: &str) -> Self::Node;
    fn create_processing_instruction(&mut self, target: &str, data: &str) -> Self::Node;
    fn append_doctype(&mut self, name: &str, public_id: &str, system_id: &str);
    fn insert(&mut self, parent: Self::Node, child: Self::Node, before: Option<Self::Node>);
    fn insert_text(&mut self, parent: Self::Node, before: Option<Self::Node>, text: &str);
    fn parent(&self, node: Self::Node) -> Option<Self::Node>;
    fn detach(&mut self, node: Self::Node);
    fn reparent_children(&mut self, from: Self::Node, to: Self::Node);
    fn add_attributes_if_missing(&mut self, element: Self::Node, attributes: Attributes<'_>);
    fn template_contents(&mut self, template: Self::Node) -> Self::Node;
    fn set_quirks_mode(&mut self, mode: QuirksMode);
}
