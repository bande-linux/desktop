use bande_mail_core::accounts::Account;
use bande_mail_core::error::{Error, Result};
use bande_mail_core::imap::{Flag, Folder, Role, Selected, Session};

use crate::note::{self, Note};
use crate::store::{FolderState, Store};

pub const ROOT: &str = "Notes";
const FETCH_BATCH: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub root: String,
    pub folders: Vec<(String, String, usize)>,
    pub trash: Option<String>,
}

fn leaf(folder: &Folder) -> &str {
    &folder.name
}

fn is_root(folder: &Folder) -> bool {
    let parent = folder
        .delimiter
        .and_then(|delimiter| folder.path.rsplit_once(delimiter))
        .map(|(parent, _)| parent);
    leaf(folder).eq_ignore_ascii_case(ROOT)
        && parent.is_none_or(|parent| parent.eq_ignore_ascii_case("INBOX"))
}

pub fn tree(folders: &[Folder]) -> Option<Tree> {
    let root = folders
        .iter()
        .filter(|folder| folder.selectable && is_root(folder))
        .min_by_key(|folder| folder.path.len())?;
    let mut tree = vec![(root.path.clone(), ROOT.to_owned(), 0)];
    if let Some(delimiter) = root.delimiter {
        let prefix = format!("{}{delimiter}", root.path);
        tree.extend(
            folders
                .iter()
                .filter(|folder| folder.selectable && folder.path.starts_with(&prefix))
                .map(|folder| {
                    let depth = folder.path[prefix.len()..].matches(delimiter).count() + 1;
                    (folder.path.clone(), folder.name.clone(), depth)
                }),
        );
    }
    Some(Tree {
        root: root.path.clone(),
        folders: tree,
        trash: folders
            .iter()
            .find(|folder| folder.role == Some(Role::Trash))
            .map(|folder| folder.path.clone()),
    })
}

fn root_to_create(folders: &[Folder]) -> String {
    let nested = folders
        .iter()
        .filter(|folder| folder.role != Some(Role::Inbox))
        .all(|folder| {
            folder.delimiter.is_some_and(|delimiter| {
                folder
                    .path
                    .to_ascii_uppercase()
                    .starts_with(&format!("INBOX{delimiter}"))
            })
        });
    match (nested, folders.iter().find_map(|folder| folder.delimiter)) {
        (true, Some(delimiter)) if folders.len() > 1 => format!("INBOX{delimiter}{ROOT}"),
        _ => ROOT.to_owned(),
    }
}

pub fn find_tree(session: &mut Session) -> Result<Option<Tree>> {
    let folders = session.list()?;
    if let Some(tree) = tree(&folders) {
        return Ok(Some(tree));
    }
    match session.create(&root_to_create(&folders)) {
        Err(Error::Refused(text)) => {
            tracing::warn!("cannot create a Notes folder: {text}");
            Ok(None)
        }
        other => {
            other?;
            Ok(tree(&session.list()?))
        }
    }
}

fn state_of(selected: &Selected) -> FolderState {
    FolderState {
        uid_validity: Some(selected.uid_validity),
        uid_next: selected.uid_next,
        present: Some(selected.exists),
        modseq: selected.highest_modseq,
    }
}

pub fn sync_account(
    session: &mut Session,
    store: &mut Store,
    account: &Account,
) -> Result<Option<bool>> {
    let Some(tree) = find_tree(session)? else {
        return Ok(None);
    };
    store.replace_folders(account.id, &tree.folders)?;
    let mut changed = false;
    for folder in store
        .folders()?
        .into_iter()
        .filter(|folder| folder.account == account.id)
    {
        changed |= sync_folder(
            session,
            store,
            account,
            folder.id,
            &folder.path,
            tree.trash.as_deref(),
        )?;
    }
    Ok(Some(changed))
}

fn sync_folder(
    session: &mut Session,
    store: &Store,
    account: &Account,
    folder: i64,
    path: &str,
    trash: Option<&str>,
) -> Result<bool> {
    let selected = session.select(path)?;
    let known_state = store.folder_state(folder)?;
    if known_state
        .uid_validity
        .is_some_and(|validity| validity != selected.uid_validity)
    {
        store.start_over(folder)?;
    }
    let removals = store.removals(folder)?;
    let dirty = store.dirty(folder)?;
    let unchanged = known_state == state_of(&selected);
    if unchanged && removals.is_empty() && dirty.is_empty() {
        return Ok(false);
    }
    for removal in &removals {
        if removal.uid_validity == selected.uid_validity {
            let removed = match (removal.trash, trash) {
                (true, Some(trash)) if trash != path => session.move_to(&[removal.uid], trash),
                _ => session.expunge(&[removal.uid]),
            };
            match removed {
                Err(Error::Refused(text)) => tracing::warn!("cannot remove a note: {text}"),
                other => other?,
            }
        }
        store.removed(removal.id)?;
    }
    let mut changed = !removals.is_empty();
    if !unchanged || !removals.is_empty() {
        changed |= fetch_changes(session, store, folder, &known_state, &selected)?;
    }
    let present = match selected.exists {
        0 => Vec::new(),
        _ => session.uids()?,
    };
    for stored in store.dirty(folder)? {
        let on_server = stored.uid.is_some_and(|uid| present.contains(&uid));
        let taken = store.uuid_taken(folder, &stored.note.uuid, stored.id)?;
        let uuid = match stored.uid.is_some() && !on_server && taken {
            true => note::new_uuid(),
            false => stored.note.uuid.clone(),
        };
        let written = Note {
            uuid: uuid.clone(),
            ..stored.note.clone()
        };
        let raw = note::write(&written, &account.name, &account.email);
        let uid = session.append(path, &[Flag::Seen], &raw)?;
        if let Some(base) = stored.uid.filter(|_| on_server) {
            session.expunge(&[base])?;
        }
        store.uploaded(stored.id, &uuid, uid, stored.note.modified)?;
        changed = true;
    }
    let state = match changed {
        true => FolderState::default(),
        false => state_of(&selected),
    };
    store.set_folder_state(folder, state)?;
    Ok(changed)
}

fn fetch_changes(
    session: &mut Session,
    store: &Store,
    folder: i64,
    known_state: &FolderState,
    selected: &Selected,
) -> Result<bool> {
    let mut server = match selected.exists {
        0 => Vec::new(),
        _ => session.uids()?,
    };
    server.sort_unstable();
    let known = store.known(folder)?;
    let mut gone = known
        .iter()
        .map(|(uid, _, _)| *uid)
        .filter(|uid| server.binary_search(uid).is_err())
        .collect::<Vec<_>>();
    let kept = known
        .iter()
        .map(|(uid, _, _)| *uid)
        .filter(|uid| server.binary_search(uid).is_ok())
        .collect::<Vec<_>>();
    if let (Some(since), Some(_)) = (known_state.modseq, selected.highest_modseq)
        && !kept.is_empty()
        && known_state.uid_validity == Some(selected.uid_validity)
    {
        gone.extend(
            session
                .fetch_flags(&kept, Some(since))?
                .into_iter()
                .filter(|fetched| fetched.flags.is_some_and(|flags| flags.deleted))
                .map(|fetched| fetched.uid),
        );
    }
    store.forget_clean(folder, &gone)?;
    let added = server
        .iter()
        .filter(|uid| !known.iter().any(|(known, _, _)| known == *uid))
        .copied()
        .collect::<Vec<_>>();
    for batch in added.chunks(FETCH_BATCH) {
        for fetched in session.fetch_bodies(batch)? {
            let Some(raw) = fetched.body else {
                continue;
            };
            match note::read(&raw) {
                Some(note) => store.arrived(folder, fetched.uid, &note)?,
                None => tracing::info!("cannot read the note with UID {}", fetched.uid),
            }
        }
    }
    Ok(!gone.is_empty() || !added.is_empty())
}
