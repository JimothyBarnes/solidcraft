//! `cam.library.*`: tool library files (the CAM work's tool item). A library is a JSON file of
//! tools (`solidcraft_cam::ToolLibrary`) kept anywhere the person likes, for example one per
//! machine; a design keeps its own copies (`cam.tool.*`), so a library can move or change without
//! changing a saved design. Reading compares the library with the design's copies, so the UI can
//! offer "library differs: update?"; importing with `replace` is that update.
use serde_json::Value;
use solidcraft_cam::{TOOL_LIBRARY_FORMAT, ToolLibrary};

use super::CommandSpec;
use super::cam::{load, store};
use crate::schema::cam::{
    CamLibraryExportParams, CamLibraryExported, CamLibraryImportParams, CamLibraryImported, CamLibraryReadParams, CamLibraryTool,
    CamLibraryToolStatus, CamLibraryTools,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session, params::bad};

/// The largest tool library file read, in bytes.
const MAX_LIBRARY_BYTES: usize = 16 * 1024 * 1024;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.library.export", "Export Tool Library", crate::checked!("cam.library.export", export)).icon("export").noundo().read_only().doc_only()
        .contract(&TypedContract::<CamLibraryExportParams>::new()).output(output_schema_for::<CamLibraryExported>)
        .params("path: a .json file, name?: the library's name, overwrite?: replace an existing file (default false) → writes the design's tools as a tool library"),
    CommandSpec::new("cam.library.read", "Read Tool Library", crate::checked!("cam.library.read", read)).icon("inspect").noundo().read_only().doc_only()
        .contract(&TypedContract::<CamLibraryReadParams>::new()).output(output_schema_for::<CamLibraryTools>)
        .params("path: a tool library .json file → name, tools: [{number, name, kind, diameter, status: new | same | differs}] against the design's tools"),
    CommandSpec::new("cam.library.import", "Import Tools", crate::checked!("cam.library.import", import)).icon("edit").doc_only()
        .contract(&TypedContract::<CamLibraryImportParams>::new()).output(output_schema_for::<CamLibraryImported>)
        .params("path, numbers?: the tools to copy (default all), replace?: update design tools that differ (default false) → added, updated, same; one undoable edit"),
];

fn path(cmd: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(bad(cmd, "invalid library path"));
    }
    if !value.to_ascii_lowercase().ends_with(".json") {
        return Err(bad(cmd, "a tool library is a .json file"));
    }
    solidcraft_io::vfs::check(value).map_err(|e| bad(cmd, e.to_string()))
}

fn reply(cmd: &str, value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| bad(cmd, e.to_string()))
}

fn read_library(cmd: &str, p: &str) -> Result<ToolLibrary> {
    path(cmd, p)?;
    let bytes = solidcraft_io::vfs::read(p, MAX_LIBRARY_BYTES).map_err(|e| bad(cmd, format!("could not read the tool library: {e}")))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| bad(cmd, "the tool library is not text"))?;
    ToolLibrary::from_json(text).map_err(|e| bad(cmd, e.to_string()))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.library.export";
    let p: CamLibraryExportParams = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    path(CMD, &p.path)?;
    let data = load(s, CMD)?;
    let library = ToolLibrary { format: TOOL_LIBRARY_FORMAT, name: p.name.unwrap_or_else(|| s.doc.name.clone()), tools: data.tools };
    let json = library.to_json().map_err(|e| bad(CMD, e.to_string()))?;
    let bytes = json.into_bytes();
    #[cfg(not(target_arch = "wasm32"))]
    let result = if p.overwrite {
        solidcraft_io::write_atomic(std::path::Path::new(&p.path), &bytes, false)
    } else {
        solidcraft_io::safe_write::create_atomic(std::path::Path::new(&p.path), &bytes)
    };
    #[cfg(target_arch = "wasm32")]
    let result = if !p.overwrite && solidcraft_io::vfs::exists(&p.path) {
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the tool library file already exists"))
    } else {
        solidcraft_io::vfs::write(&p.path, &bytes)
    };
    result.map_err(|e| bad(CMD, format!("could not export the tool library: {e}")))?;
    reply(CMD, CamLibraryExported { path: p.path, tools: library.tools.len(), bytes: bytes.len() })
}

fn status(design: &[solidcraft_cam::Tool], tool: &solidcraft_cam::Tool) -> CamLibraryToolStatus {
    match design.iter().find(|t| t.number == tool.number) {
        None => CamLibraryToolStatus::New,
        Some(t) if t == tool => CamLibraryToolStatus::Same,
        Some(_) => CamLibraryToolStatus::Differs,
    }
}

fn read(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.library.read";
    let p: CamLibraryReadParams = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let library = read_library(CMD, &p.path)?;
    let data = load(s, CMD)?;
    let tools = library
        .tools
        .iter()
        .map(|t| CamLibraryTool {
            number: t.number,
            name: t.name.clone(),
            kind: serde_json::to_value(t.kind).unwrap_or(Value::Null),
            diameter: t.geometry.diameter,
            status: status(&data.tools, t),
        })
        .collect();
    reply(CMD, CamLibraryTools { name: library.name, tools })
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.library.import";
    let p: CamLibraryImportParams = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    let library = read_library(CMD, &p.path)?;
    if let Some(numbers) = &p.numbers
        && let Some(missing) = numbers.iter().find(|n| !library.tools.iter().any(|t| t.number == **n))
    {
        return Err(bad(CMD, format!("the library has no tool {missing}")));
    }
    let mut data = load(s, CMD)?;
    let (mut added, mut updated, mut same) = (Vec::new(), Vec::new(), Vec::new());
    for tool in library.tools.into_iter().filter(|t| p.numbers.as_ref().is_none_or(|n| n.contains(&t.number))) {
        match status(&data.tools, &tool) {
            CamLibraryToolStatus::New => {
                added.push(tool.number);
                data.tools.push(tool);
            }
            CamLibraryToolStatus::Same => same.push(tool.number),
            CamLibraryToolStatus::Differs if p.replace => {
                updated.push(tool.number);
                if let Some(slot) = data.tools.iter_mut().find(|t| t.number == tool.number) {
                    *slot = tool;
                }
            }
            CamLibraryToolStatus::Differs => {
                return Err(bad(CMD, format!("the design's tool {} differs from the library's; import it with replace to update it", tool.number)));
            }
        }
    }
    if !added.is_empty() || !updated.is_empty() {
        store(s, CMD, &data)?;
    }
    reply(CMD, CamLibraryImported { added, updated, same })
}

#[cfg(test)]
#[path = "cam_library_tests.rs"]
mod tests;
