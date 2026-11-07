use std::path::Path;
use std::sync::Arc;

use bande_search::{Restored, Stamp};
use tantivy::collector::{FilterCollector, TopDocs};
use tantivy::query::{BooleanQuery, Occur, PhrasePrefixQuery, PhraseQuery, Query, TermQuery};
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TantivyDocument,
    TextFieldIndexing, TextOptions, Value,
};
use tantivy::{DocAddress, Index, IndexReader, ReloadPolicy, Term, doc};

use crate::tokens::{ANALYZER, analyzer, words};

const MAX_PREFIX_WORDS: usize = 200;

#[derive(Clone, Copy)]
pub struct Fields {
    pub entry: Field,
    pub parent: Field,
    pub name: Field,
    pub folder: Field,
    pub inode: Field,
    pub modified: Field,
    pub size: Field,
    pub imported: Field,
    pub content: Field,
    pub body: Field,
    pub item: Field,
    pub domain: Field,
    pub kind: Field,
    pub title: Field,
    pub detail: Field,
    pub target: Field,
}

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let body_options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(ANALYZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let fields = Fields {
        entry: builder.add_u64_field("entry", INDEXED | FAST),
        parent: builder.add_u64_field("parent", FAST),
        name: builder.add_text_field("name", STORED),
        folder: builder.add_bool_field("folder", FAST),
        inode: builder.add_u64_field("inode", FAST),
        modified: builder.add_i64_field("modified", FAST),
        size: builder.add_u64_field("size", FAST),
        imported: builder.add_bool_field("imported", FAST),
        content: builder.add_u64_field("content", INDEXED | FAST),
        body: builder.add_text_field("body", body_options),
        item: builder.add_text_field("item", STRING | STORED),
        domain: builder.add_text_field("domain", STRING | STORED),
        kind: builder.add_text_field("kind", STORED),
        title: builder.add_text_field("title", STORED),
        detail: builder.add_text_field("detail", STORED),
        target: builder.add_text_field("target", STORED),
    };
    (builder.build(), fields)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub parent: u32,
    pub name: String,
    pub folder: bool,
    pub stamp: Stamp,
}

pub fn entry_document(fields: &Fields, entry: &Entry) -> TantivyDocument {
    doc!(
        fields.entry => u64::from(entry.id),
        fields.parent => u64::from(entry.parent),
        fields.name => entry.name.as_str(),
        fields.folder => entry.folder,
        fields.inode => entry.stamp.inode,
        fields.modified => entry.stamp.modified,
        fields.size => entry.stamp.size,
        fields.imported => entry.stamp.imported,
    )
}

pub enum Hit {
    File { id: u32, relevance: f32 },
    Item(Item),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub domain: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub target: String,
    pub relevance: f32,
}

pub struct Store {
    pub index: Index,
    pub fields: Fields,
    reader: IndexReader,
}

impl Store {
    pub fn open(folder: &Path) -> tantivy::Result<Self> {
        let (schema, fields) = schema();
        let index = match Index::open_in_dir(folder) {
            Ok(index) if index.schema() == schema => index,
            _ => {
                wipe(folder);
                Index::create_in_dir(folder, schema)?
            }
        };
        index.tokenizers().register(ANALYZER, analyzer());
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            index,
            fields,
            reader,
        })
    }

    pub fn reader(&self) -> IndexReader {
        self.reader.clone()
    }

    pub fn entries(&self) -> tantivy::Result<Vec<Restored>> {
        let searcher = self.reader.searcher();
        let mut entries = Vec::new();
        for (ordinal, segment) in searcher.segment_readers().iter().enumerate() {
            let columns = segment.fast_fields();
            let entry = columns.u64("entry")?;
            let parent = columns.u64("parent")?;
            let folder = columns.bool("folder")?;
            let inode = columns.u64("inode")?;
            let modified = columns.i64("modified")?;
            let size = columns.u64("size")?;
            let imported = columns.bool("imported")?;
            for document in segment.doc_ids_alive() {
                let (Some(id), Some(parent)) = (entry.first(document), parent.first(document))
                else {
                    continue;
                };
                let (Ok(id), Ok(parent)) = (u32::try_from(id), u32::try_from(parent)) else {
                    continue;
                };
                let stored: TantivyDocument =
                    searcher.doc(DocAddress::new(ordinal as u32, document))?;
                let Some(name) = text_of(&stored, self.fields.name) else {
                    continue;
                };
                entries.push(Restored {
                    id,
                    parent,
                    name,
                    folder: folder.first(document).unwrap_or(false),
                    stamp: Stamp {
                        inode: inode.first(document).unwrap_or(0),
                        modified: modified.first(document).unwrap_or(0),
                        size: size.first(document).unwrap_or(0),
                        imported: imported.first(document).unwrap_or(false),
                    },
                });
            }
        }
        Ok(entries)
    }

    pub fn stored_title(&self, key: &str) -> Option<String> {
        let searcher = self.reader.searcher();
        let query = TermQuery::new(
            Term::from_field_text(self.fields.item, key),
            IndexRecordOption::Basic,
        );
        let found = searcher.search(&query, &TopDocs::with_limit(1)).ok()?;
        let (_, address) = found.into_iter().next()?;
        let stored: TantivyDocument = searcher.doc(address).ok()?;
        text_of(&stored, self.fields.title)
    }

    pub fn search(
        &self,
        text: &str,
        limit: usize,
        within: Option<Arc<Vec<bool>>>,
    ) -> tantivy::Result<Vec<Hit>> {
        let Some(query) = content_query(self.fields.body, text) else {
            return Ok(Vec::new());
        };
        let searcher = self.reader.searcher();
        let top = TopDocs::with_limit(limit.max(1));
        let found = match within {
            Some(allowed) => searcher.search(
                &query,
                &FilterCollector::new(
                    "content".to_owned(),
                    move |id: u64| {
                        usize::try_from(id)
                            .ok()
                            .and_then(|id| allowed.get(id).copied())
                            .unwrap_or(false)
                    },
                    top,
                ),
            )?,
            None => searcher.search(&query, &top)?,
        };
        let mut hits = Vec::with_capacity(found.len());
        for (relevance, address) in found {
            let segment = searcher.segment_reader(address.segment_ord);
            let id = segment.fast_fields().u64("content")?.first(address.doc_id);
            if let Some(id) = id.and_then(|id| u32::try_from(id).ok()) {
                hits.push(Hit::File { id, relevance });
                continue;
            }
            let stored: TantivyDocument = searcher.doc(address)?;
            let field = |field| text_of(&stored, field).unwrap_or_default();
            if field(self.fields.kind).is_empty() {
                continue;
            }
            hits.push(Hit::Item(Item {
                domain: field(self.fields.domain),
                kind: field(self.fields.kind),
                title: field(self.fields.title),
                detail: field(self.fields.detail),
                target: field(self.fields.target),
                relevance,
            }));
        }
        Ok(hits)
    }
}

fn text_of(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn wipe(folder: &Path) {
    let Ok(files) = std::fs::read_dir(folder) else {
        return;
    };
    for file in files.flatten() {
        if let Err(error) = std::fs::remove_file(file.path()) {
            tracing::warn!("cannot clear the old search index: {error}");
        }
    }
}

struct Group {
    words: Vec<String>,
    prefix: bool,
}

fn groups(text: &str) -> Vec<Group> {
    let mut groups = Vec::new();
    let mut quoted = false;
    for (index, piece) in text.split('"').enumerate() {
        let last_piece = index == text.split('"').count() - 1;
        if quoted {
            groups.push(Group {
                words: words(piece),
                prefix: false,
            });
        } else {
            let spaces_after = piece.ends_with(char::is_whitespace);
            let pieces: Vec<&str> = piece.split_whitespace().collect();
            for (at, word) in pieces.iter().enumerate() {
                groups.push(Group {
                    words: words(word),
                    prefix: last_piece && !spaces_after && at == pieces.len() - 1,
                });
            }
        }
        quoted = !quoted;
    }
    groups.retain(|group| !group.words.is_empty());
    groups
}

fn group_query(body: Field, group: &Group) -> Option<Box<dyn Query>> {
    let terms: Vec<Term> = group
        .words
        .iter()
        .map(|word| Term::from_field_text(body, word))
        .collect();
    let single = group.words.first().filter(|_| group.words.len() == 1);
    if group.prefix {
        if single.is_some_and(|word| word.len() < 2) {
            return None;
        }
        let mut query = PhrasePrefixQuery::new(terms);
        query.set_max_expansions(MAX_PREFIX_WORDS as u32);
        return Some(Box::new(query));
    }
    match terms.len() {
        0 => None,
        1 => terms.into_iter().next().map(|term| {
            Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs)) as Box<dyn Query>
        }),
        _ => Some(Box::new(PhraseQuery::new(terms))),
    }
}

fn content_query(body: Field, text: &str) -> Option<Box<dyn Query>> {
    let clauses: Vec<(Occur, Box<dyn Query>)> = groups(text)
        .iter()
        .filter_map(|group| group_query(body, group))
        .map(|query| (Occur::Must, query))
        .collect();
    (!clauses.is_empty()).then(|| Box::new(BooleanQuery::new(clauses)) as Box<dyn Query>)
}
