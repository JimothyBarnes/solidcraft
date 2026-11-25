//! `cam.posts.*`: the post library (decision 0039 of the CAM work). A post is a file in our own
//! JSON post format (`solidcraft_cam::Post`): the three built into the app, and every post file in
//! the post folder, by default `cam/posts` in SolidCraft's per-user data folder, where a machine
//! maker's or a shop's posts go. A file is read each time it is listed or posted with, so a post
//! edited in the folder is used as it now is. `cam.post` takes a built-in post by its id and a
//! post file by its path.
use serde_json::Value;
use solidcraft_cam::{BUILTIN_POSTS, Post, PropertyKind};

use super::CommandSpec;
use crate::schema::cam::{CamPostAdded, CamPostChoice, CamPostUnread, CamPostsAddParams, CamPostsListParams};
pub use crate::schema::cam::{CamPostEntry, CamPostLibrary, CamPostProperty};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session, params::bad};

/// The longest text a post's text property holds, in characters.
pub const MAX_POST_PROPERTY_TEXT: usize = solidcraft_cam::MAX_PROPERTY_TEXT;
/// The largest post file read, in bytes.
const MAX_POST_BYTES: usize = 1024 * 1024;
/// The most files a post folder may hold.
const MAX_POST_FILES: usize = 1000;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.posts.add", "Add Post", crate::checked!("cam.posts.add", add)).icon("import").noundo().read_only().doc_only()
        .contract(&TypedContract::<CamPostsAddParams>::new()).output(output_schema_for::<CamPostAdded>)
        .params("file: a post .json file, folder?: the post folder (default cam/posts in SolidCraft's data folder), replace?: replace the folder's post of the same id (default false) → post: its path in the folder, what cam.post takes; id, name, vendor, version, replaced?: the version it replaced"),
    CommandSpec::new("cam.posts.list", "List Posts", crate::checked!("cam.posts.list", list)).icon("inspect").noundo().read_only().doc_only()
        .contract(&TypedContract::<CamPostsListParams>::new()).output(output_schema_for::<CamPostLibrary>)
        .params("folder?: the post folder (default cam/posts in SolidCraft's data folder) → posts: [{post: what cam.post takes, id, name, vendor, version, description, file?, optional_stop, properties: [{id, label, kind: bool | number | choice | text, default, choices?, min?, max?, decimals?, description?}]}], the built-in ones first; unread: [{file, why}], the folder's .json files that are no post"),
];

/// The post folder when none is given: `cam/posts` in SolidCraft's per-user data folder; none
/// where the system has no such folder.
pub fn manufacture_post_folder() -> Option<String> {
    crate::recovery::data_dir().map(|dir| dir.join("cam").join("posts").to_string_lossy().into_owned())
}

fn file_path(cmd: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(bad(cmd, "invalid post file path"));
    }
    if !value.to_ascii_lowercase().ends_with(".json") {
        return Err(bad(cmd, "a post file is a .json file"));
    }
    solidcraft_io::vfs::check(value).map_err(|e| bad(cmd, e.to_string()))
}

fn folder_path(cmd: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(bad(cmd, "invalid post folder path"));
    }
    solidcraft_io::vfs::check(value).map_err(|e| bad(cmd, e.to_string()))
}

fn reply(cmd: &str, value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| bad(cmd, e.to_string()))
}

/// The post file at `path`, read and checked.
fn read_post(cmd: &str, path: &str) -> Result<(Post, Vec<u8>)> {
    file_path(cmd, path)?;
    let bytes = solidcraft_io::vfs::read(path, MAX_POST_BYTES).map_err(|e| bad(cmd, format!("could not read the post file: {e}")))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| bad(cmd, "the post file is not text"))?;
    let post = Post::from_json(text).map_err(|e| bad(cmd, e.to_string()))?;
    Ok((post, bytes))
}

/// The post `post` names: a built-in post's id, or a post file's path.
pub(super) fn named(cmd: &str, post: &str) -> Result<Post> {
    if BUILTIN_POSTS.iter().any(|(id, _)| *id == post) {
        return Post::builtin(post).map_err(|e| bad(cmd, e.to_string()));
    }
    if !post.to_ascii_lowercase().ends_with(".json") {
        let ids = BUILTIN_POSTS.iter().map(|(id, _)| *id).collect::<Vec<_>>().join(", ");
        return Err(bad(
            cmd,
            format!("no post '{post}' (built in: {ids}; a post file is named by its path, and cam.posts.list lists the post folder's)"),
        ));
    }
    read_post(cmd, post).map(|(post, _)| post)
}

fn entry(post: &Post, named: String, file: Option<String>) -> CamPostEntry {
    let kind = |kind: PropertyKind| match kind {
        PropertyKind::Bool => "bool",
        PropertyKind::Number => "number",
        PropertyKind::Choice => "choice",
        PropertyKind::Text => "text",
    };
    CamPostEntry {
        post: named,
        id: post.id.clone(),
        name: post.name.clone(),
        vendor: post.vendor.clone(),
        version: post.version.clone(),
        description: post.description.clone(),
        file,
        optional_stop: post.optional_stop.is_some(),
        properties: post
            .properties
            .iter()
            .map(|p| CamPostProperty {
                id: p.id.clone(),
                label: p.label.clone(),
                kind: kind(p.kind).into(),
                default: p.default.clone(),
                choices: p.choices.iter().map(|c| CamPostChoice { value: c.value.clone(), label: c.label.clone() }).collect(),
                min: p.min,
                max: p.max,
                decimals: p.decimals,
                description: p.description.clone(),
            })
            .collect(),
    }
}

/// The posts built in, then every post file in `folder` (none when it is not there), and the
/// `.json` files in it that are no post, with why.
fn library(cmd: &str, folder: Option<&str>) -> CamPostLibrary {
    let mut posts: Vec<CamPostEntry> =
        BUILTIN_POSTS.iter().filter_map(|(id, _)| Post::builtin(id).ok().map(|post| entry(&post, (*id).to_string(), None))).collect();
    let mut unread = Vec::new();
    let listed = folder.map(|dir| (dir, folder_path(cmd, dir).map(|()| solidcraft_io::vfs::exists(dir))));
    match listed {
        None | Some((_, Ok(false))) => {}
        Some((dir, Err(e))) => unread.push(CamPostUnread { file: dir.to_string(), why: super::cam::said(e) }),
        Some((dir, Ok(true))) => match solidcraft_io::vfs::files_in(dir, MAX_POST_FILES) {
            Ok(files) => {
                for file in files.into_iter().filter(|file| file.to_ascii_lowercase().ends_with(".json")) {
                    match read_post(cmd, &file) {
                        Ok((post, _)) => posts.push(entry(&post, file.clone(), Some(file))),
                        Err(e) => unread.push(CamPostUnread { file, why: super::cam::said(e) }),
                    }
                }
            }
            Err(e) => unread.push(CamPostUnread { file: dir.to_string(), why: format!("could not list the post folder: {e}") }),
        },
    }
    CamPostLibrary { folder: folder.map(str::to_string), posts, unread }
}

fn list(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.posts.list";
    let p: CamPostsListParams = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let folder = p.folder.or_else(manufacture_post_folder);
    reply(CMD, library(CMD, folder.as_deref()))
}

/// The post library as `cam.posts.list` gives it for `folder` (none: only the posts built in),
/// for Post Process to offer. It reads the folder and changes nothing.
pub fn manufacture_post_library(folder: Option<&str>) -> CamPostLibrary {
    library("cam.posts.list", folder)
}

fn add(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.posts.add";
    let p: CamPostsAddParams = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let (post, bytes) = read_post(CMD, &p.file)?;
    let folder = p
        .folder
        .or_else(manufacture_post_folder)
        .ok_or_else(|| bad(CMD, "this system has no folder for SolidCraft's own files: give the post folder"))?;
    folder_path(CMD, &folder)?;
    let target = std::path::Path::new(&folder).join(format!("{}.json", post.id)).to_string_lossy().into_owned();
    let replaced = if std::path::Path::new(&p.file) == std::path::Path::new(&target) {
        // It is the folder's own file already.
        None
    } else if solidcraft_io::vfs::exists(&target) {
        let old = read_post(CMD, &target).map(|(old, _)| old.version).unwrap_or_default();
        if !p.replace {
            let version = if old.is_empty() { String::new() } else { format!(" (version {old})") };
            return Err(bad(CMD, format!("the post folder has {}.json already{version}; add it with replace to replace it", post.id)));
        }
        Some(old)
    } else {
        None
    };
    if replaced.is_some() || !solidcraft_io::vfs::exists(&target) {
        solidcraft_io::vfs::create_dir_all(&folder).map_err(|e| bad(CMD, format!("could not make the post folder: {e}")))?;
        solidcraft_io::vfs::write(&target, &bytes).map_err(|e| bad(CMD, format!("could not write the post file: {e}")))?;
    }
    reply(CMD, CamPostAdded { post: target, id: post.id, name: post.name, vendor: post.vendor, version: post.version, replaced })
}
