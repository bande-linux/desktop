use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, TextAnalyzer, Token, TokenStream, Tokenizer,
};

pub const ANALYZER: &str = "bande";
const LONGEST_WORD: usize = 64;

#[derive(Clone, Default)]
struct Words {
    token: Token,
}

struct WordStream<'a> {
    text: &'a str,
    at: usize,
    position: usize,
    token: &'a mut Token,
}

fn stands_alone(character: char) -> bool {
    matches!(character,
        '\u{3040}'..='\u{30ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}'
        | '\u{f900}'..='\u{faff}'
        | '\u{20000}'..='\u{2fa1f}')
}

impl Tokenizer for Words {
    type TokenStream<'a> = WordStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> WordStream<'a> {
        self.token.reset();
        WordStream {
            text,
            at: 0,
            position: 0,
            token: &mut self.token,
        }
    }
}

impl TokenStream for WordStream<'_> {
    fn advance(&mut self) -> bool {
        let rest = self.text.get(self.at..).unwrap_or_default();
        let Some((skip, first)) = rest
            .char_indices()
            .find(|(_, character)| character.is_alphanumeric())
        else {
            self.at = self.text.len();
            return false;
        };
        let start = self.at + skip;
        let mut end = start + first.len_utf8();
        if !stands_alone(first) {
            let word = self.text.get(end..).unwrap_or_default();
            end += word
                .char_indices()
                .find(|(_, character)| !character.is_alphanumeric() || stands_alone(*character))
                .map_or(word.len(), |(index, _)| index);
        }
        self.at = end;
        self.token.offset_from = start;
        self.token.offset_to = end;
        self.token.position = self.position;
        self.token.position_length = 1;
        self.token.text.clear();
        self.token
            .text
            .push_str(self.text.get(start..end).unwrap_or_default());
        self.position += 1;
        true
    }

    fn token(&self) -> &Token {
        self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        self.token
    }
}

pub fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(Words::default())
        .filter(RemoveLongFilter::limit(LONGEST_WORD))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build()
}

pub fn words(text: &str) -> Vec<String> {
    let mut analyzer = analyzer();
    let mut stream = analyzer.token_stream(text);
    let mut words = Vec::new();
    while stream.advance() {
        words.push(stream.token().text.clone());
    }
    words
}
