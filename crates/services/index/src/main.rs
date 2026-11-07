use std::process::ExitCode;

use bande_content::Importer;

const IMPORT_FLAG: &str = "--import";

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some(IMPORT_FLAG) {
        return match arguments.next().as_deref().and_then(Importer::from_keyword) {
            Some(importer) => bande_indexer::import_stdin(importer),
            None => ExitCode::FAILURE,
        };
    }
    bande_log::init_quieting("bande-indexer", &["tantivy"]);
    match bande_indexer::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
