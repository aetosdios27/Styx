use std::collections::BTreeMap;

use crate::bencode::BencodeValue;
use crate::info_hash_v2::InfoHashV2;

/// A single file entry in a v2 file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2FileEntry {
    pub length: u64,
    pub pieces_root: Option<InfoHashV2>,
}

/// A node in the v2 file tree — either a file or a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2FileTreeNode {
    File(V2FileEntry),
    Directory(BTreeMap<Vec<u8>, V2FileTreeNode>),
}

/// A flattened file entry with full path.
#[derive(Debug, Clone)]
pub struct V2FlatFile {
    pub path_components: Vec<Vec<u8>>,
    pub entry: V2FileEntry,
}

/// The v2 file tree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2FileTree {
    pub root: BTreeMap<Vec<u8>, V2FileTreeNode>,
}

/// Errors from v2 file tree validation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FileTreeError {
    #[error("file tree root is a file, not a directory")]
    RootIsFile,
    #[error("empty file path component")]
    EmptyComponent,
    #[error("path traversal component: {0:?}")]
    PathTraversal(Vec<u8>),
    #[error("path component contains separator: {0:?}")]
    PathSeparator(Vec<u8>),
    #[error("file tree contains no files")]
    NoFiles,
    #[error("duplicate file path")]
    DuplicatePath,
}

impl V2FileTree {
    /// Parse from a bencoded value.
    /// The value must be a dictionary (BTreeMap of byte string keys to nodes).
    pub fn from_bencode(value: &BencodeValue) -> Result<Self, FileTreeError> {
        let dict = match value {
            BencodeValue::Dict(d) => d,
            _ => return Err(FileTreeError::RootIsFile),
        };

        let root = parse_file_tree_dict(dict)?;

        if root.contains_key(&b""[..]) {
            return Err(FileTreeError::RootIsFile);
        }

        Ok(Self { root })
    }

    /// Validate and flatten the tree into an ordered list of files.
    pub fn flatten(&self) -> Result<Vec<V2FlatFile>, FileTreeError> {
        let mut files = Vec::new();
        let mut path_stack: Vec<Vec<u8>> = Vec::new();
        self.flatten_recursive(&self.root, &mut path_stack, &mut files)?;

        if files.is_empty() {
            return Err(FileTreeError::NoFiles);
        }

        let mut seen = std::collections::HashSet::new();
        for f in &files {
            if !seen.insert(f.path_components.clone()) {
                return Err(FileTreeError::DuplicatePath);
            }
        }

        Ok(files)
    }

    fn flatten_recursive(
        &self,
        node: &BTreeMap<Vec<u8>, V2FileTreeNode>,
        path_stack: &mut Vec<Vec<u8>>,
        files: &mut Vec<V2FlatFile>,
    ) -> Result<(), FileTreeError> {
        for (key, child) in node {
            if key.is_empty() {
                continue;
            }
            if key == b"." || key == b".." {
                return Err(FileTreeError::PathTraversal(key.clone()));
            }
            if key.contains(&b'/') || key.contains(&b'\\') {
                return Err(FileTreeError::PathSeparator(key.clone()));
            }

            match child {
                V2FileTreeNode::File(entry) => {
                    path_stack.push(key.clone());
                    files.push(V2FlatFile {
                        path_components: path_stack.clone(),
                        entry: entry.clone(),
                    });
                    path_stack.pop();
                }
                V2FileTreeNode::Directory(children) => {
                    path_stack.push(key.clone());
                    self.flatten_recursive(children, path_stack, files)?;
                    path_stack.pop();
                }
            }
        }
        Ok(())
    }
}

fn parse_file_tree_node(value: &BencodeValue) -> Result<V2FileTreeNode, FileTreeError> {
    let dict = match value {
        BencodeValue::Dict(d) => d,
        _ => return Err(FileTreeError::RootIsFile),
    };

    if let Some(entry_value) = dict.get(&b""[..]) {
        let entry_dict = match entry_value {
            BencodeValue::Dict(d) => d,
            _ => return Err(FileTreeError::RootIsFile),
        };

        let length = entry_dict
            .get(b"length".as_slice())
            .and_then(|v| match v {
                BencodeValue::Integer(i) => Some(*i as u64),
                _ => None,
            })
            .ok_or(FileTreeError::NoFiles)?;

        let pieces_root = entry_dict
            .get(b"pieces root".as_slice())
            .and_then(|v| match v {
                BencodeValue::Bytes(b) if b.len() == 32 => {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(b);
                    Some(InfoHashV2::new(arr))
                }
                _ => None,
            });

        return Ok(V2FileTreeNode::File(V2FileEntry {
            length,
            pieces_root,
        }));
    }

    let children = parse_file_tree_dict(dict)?;
    Ok(V2FileTreeNode::Directory(children))
}

fn parse_file_tree_dict(
    dict: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<BTreeMap<Vec<u8>, V2FileTreeNode>, FileTreeError> {
    let mut children = BTreeMap::new();
    for (key, value) in dict {
        children.insert(key.clone(), parse_file_tree_node(value)?);
    }
    Ok(children)
}
