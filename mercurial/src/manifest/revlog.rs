// Copyright (c) 2004-present, Facebook, Inc.
// All Rights Reserved.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2 or any later version.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::str;
use std::vec;

use futures::{Async, Poll};
use futures::future::{Future, IntoFuture};
use futures::stream::Stream;
use futures_ext::{BoxFuture, BoxStream, FutureExt, StreamExt};

use errors::*;
use failure;
use file;
use mercurial_types::{Blob, BlobNode, MPath, MPathElement, NodeHash, Parents, RepoPath};
use mercurial_types::manifest::{Content, Entry, Manifest, Type};
use mercurial_types::nodehash::EntryId;

use RevlogRepo;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Details {
    entryid: EntryId,
    flag: Type,
}

/// Revlog Manifest v1
#[derive(Debug, PartialEq)]
pub struct RevlogManifest {
    // This is None for testing only -- the public API ensures `repo` always exists.
    repo: Option<RevlogRepo>,
    parents: Parents,
    content: ManifestContent,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ManifestContent {
    pub files: BTreeMap<MPath, Details>,
}

impl ManifestContent {
    pub fn new_empty() -> Self {
        Self {
            files: BTreeMap::new(),
        }
    }

    // Each manifest revision contains a list of the file revisions in each changeset, in the form:
    //
    // <filename>\0<hex file revision id>[<flags>]\n
    //
    // Source: mercurial/parsers.c:parse_manifest()
    //
    // NB: filenames are sequences of non-zero bytes, not strings
    fn parse_impl(data: &[u8], prefix: Option<&MPath>) -> Result<BTreeMap<MPath, Details>> {
        let mut files = BTreeMap::new();

        for line in data.split(|b| *b == b'\n') {
            if line.len() == 0 {
                break;
            }

            let (name, rest) = match find(line, &0) {
                None => bail_msg!("Malformed entry: no \\0"),
                Some(nil) => {
                    let (name, rest) = line.split_at(nil);
                    if let Some((_, hash)) = rest.split_first() {
                        (name, hash)
                    } else {
                        bail_msg!("Malformed entry: no hash");
                    }
                }
            };

            let path = if let Some(prefix) = prefix {
                prefix.join(&MPath::new(name).context("invalid path in manifest")?)
            } else {
                MPath::new(name).context("invalid path in manifest")?
            };
            let details = Details::parse(rest)?;

            // XXX check path > last entry in files
            files.insert(path, details);
        }

        Ok(files)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        Ok(Self {
            files: Self::parse_impl(data, None)?,
        })
    }

    pub fn parse_with_prefix(data: &[u8], prefix: &MPath) -> Result<Self> {
        Ok(Self {
            files: Self::parse_impl(data, Some(prefix))?,
        })
    }

    pub fn generate<W: Write>(&self, out: &mut W) -> io::Result<()> {
        for (ref k, ref v) in &self.files {
            k.generate(out)?;
            out.write(&b"\0"[..])?;
            v.generate(out)?;
            out.write(&b"\n"[..])?;
        }
        Ok(())
    }
}

impl RevlogManifest {
    pub fn new(repo: RevlogRepo, node: BlobNode) -> Result<RevlogManifest> {
        node.as_blob()
            .as_slice()
            .ok_or(failure::err_msg("node missing data"))
            .and_then(|blob| Self::parse(Some(repo), node.parents(), blob))
    }

    fn parse(repo: Option<RevlogRepo>, parents: &Parents, data: &[u8]) -> Result<RevlogManifest> {
        // This is private because it allows one to create a RevlogManifest with repo set to None.
        ManifestContent::parse(data).map(|content| RevlogManifest {
            repo,
            parents: parents.clone(),
            content,
        })
    }

    fn parse_with_prefix(
        repo: Option<RevlogRepo>,
        parents: &Parents,
        data: &[u8],
        prefix: &MPath,
    ) -> Result<RevlogManifest> {
        // This is private because it allows one to create a RevlogManifest with repo set to None.
        ManifestContent::parse_with_prefix(data, prefix).map(|content| RevlogManifest {
            repo,
            parents: parents.clone(),
            content,
        })
    }

    pub fn parents(&self) -> &Parents {
        &self.parents
    }

    pub fn generate<W: Write>(&self, out: &mut W) -> io::Result<()> {
        self.content.generate(out)
    }

    pub fn lookup(&self, path: &MPath) -> Option<&Details> {
        self.content.files.get(path)
    }

    pub fn manifest(&self) -> Vec<(&MPath, &Details)> {
        self.content.files.iter().collect()
    }
}

impl Details {
    pub fn new(entryid: EntryId, flag: Type) -> Self {
        Self { entryid, flag }
    }

    fn parse(data: &[u8]) -> Result<Details> {
        ensure_msg!(data.len() >= 40, "hash too small: {:?}", data);

        let (hash, flags) = data.split_at(40);
        let hash = str::from_utf8(hash)
            .map_err(|err| Error::from(err))
            .and_then(|hash| hash.parse::<NodeHash>())
            .with_context(|_| format!("malformed hash: {:?}", hash))?;
        let entryid = EntryId::new(hash);

        ensure_msg!(flags.len() <= 1, "More than 1 flag: {:?}", flags);

        let flag = if flags.len() == 0 {
            Type::File
        } else {
            match flags[0] {
                b'l' => Type::Symlink,
                b'x' => Type::Executable,
                b't' => Type::Tree,
                unk => bail_msg!("Unknown flag {}", unk),
            }
        };

        Ok(Details {
            entryid: entryid,
            flag: flag,
        })
    }

    fn generate<W: Write>(&self, out: &mut W) -> io::Result<()> {
        write!(out, "{}{}", self.entryid.into_nodehash(), self.flag)
    }

    pub fn entryid(&self) -> &EntryId {
        &self.entryid
    }

    pub fn flag(&self) -> Type {
        self.flag
    }

    pub fn is_symlink(&self) -> bool {
        self.flag == Type::Symlink
    }

    pub fn is_tree(&self) -> bool {
        self.flag == Type::Tree
    }

    pub fn is_executable(&self) -> bool {
        self.flag == Type::Executable
    }

    pub fn is_file(&self) -> bool {
        self.flag == Type::File
    }
}

fn find<T>(haystack: &[T], needle: &T) -> Option<usize>
where
    T: PartialEq,
{
    haystack.iter().position(|e| e == needle)
}

pub struct RevlogEntry {
    repo: RevlogRepo,
    path: RepoPath,
    name: Option<MPathElement>,
    details: Details,
}

pub struct RevlogListStream(vec::IntoIter<(MPath, Details)>, RevlogRepo);

impl Stream for RevlogListStream {
    type Item = RevlogEntry;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Error> {
        let v = self.0
            .next()
            .map(|(path, details)| RevlogEntry::new(self.1.clone(), path, details));
        match v {
            Some(v) => v.map(|x| Async::Ready(Some(x))),
            None => Ok(Async::Ready(None)),
        }
    }
}

impl Manifest for RevlogManifest {
    fn lookup(&self, path: &MPath) -> BoxFuture<Option<Box<Entry + Sync>>, Error> {
        let repo = self.repo.as_ref().expect("missing repo").clone();
        let res = RevlogManifest::lookup(self, path)
            .map(|details| RevlogEntry::new(repo, path.clone(), *details));

        match res {
            Some(v) => v.map(|e| Some(e.boxed())).into_future().boxify(),
            None => Ok(None).into_future().boxify(),
        }
    }

    fn list(&self) -> BoxStream<Box<Entry + Sync>, Error> {
        let v: Vec<_> = self.manifest()
            .into_iter()
            .map(|(p, d)| (p.clone(), *d))
            .collect();
        RevlogListStream(
            v.into_iter(),
            self.repo.as_ref().expect("missing repo").clone(),
        ).map(|e| e.boxed())
            .boxify()
    }
}

impl RevlogEntry {
    fn new(repo: RevlogRepo, path: MPath, details: Details) -> Result<Self> {
        let name = (&path).into_iter().next_back().map(|path| path.clone());
        let path = match details.flag() {
            Type::Tree => RepoPath::dir(path)
                .with_context(|_| ErrorKind::Path("error while creating RepoPath".into()))?,
            _ => RepoPath::file(path)
                .with_context(|_| ErrorKind::Path("error while creating RepoPath".into()))?,
        };

        // For revlog we still need to store full path, because full path is used to find revlog
        // file
        Ok(RevlogEntry {
            repo,
            path,
            name,
            details,
        })
    }

    fn get_path(&self) -> &RepoPath {
        &self.path
    }
}

impl Entry for RevlogEntry {
    fn get_type(&self) -> Type {
        self.details.flag()
    }

    fn get_parents(&self) -> BoxFuture<Parents, Error> {
        let revlog = self.repo.get_path_revlog(self.get_path());
        let nodeid = self.get_hash().into_nodehash();
        revlog
            .and_then(|revlog| revlog.get_rev_by_nodeid(&nodeid))
            .map(|node| *node.parents())
            .into_future()
            .boxify()
    }

    fn get_raw_content(&self) -> BoxFuture<Blob, Error> {
        let revlog = self.repo.get_path_revlog(self.get_path());
        let nodeid = self.get_hash().into_nodehash();
        revlog
            .and_then(|revlog| revlog.get_rev_by_nodeid(&nodeid))
            .map(|node| node.as_blob().clone())
            .map_err(|err| {
                err.context(format_err!(
                    "Can't get content for {} node {}",
                    self.get_path(),
                    nodeid
                ))
            })
            .map_err(Error::from)
            .into_future()
            .boxify()
    }

    fn get_content(&self) -> BoxFuture<Content, Error> {
        let revlog = self.repo.get_path_revlog(self.get_path());
        let nodeid = self.get_hash().into_nodehash();

        revlog
            .and_then(|revlog| revlog.get_rev_by_nodeid(&nodeid))
            .and_then(|node| {
                let data = node.as_blob();
                match self.get_type() {
                    // Mercurial file blob can have metadata, but tree manifest can't
                    // So strip metdata from everything except for Tree
                    Type::File => Ok(Content::File(strip_file_metadata(data))),
                    Type::Executable => Ok(Content::Executable(strip_file_metadata(data))),
                    Type::Symlink => {
                        let data = strip_file_metadata(data);
                        let data = data.as_slice()
                            .ok_or(failure::err_msg("missing symlink blob data"))?;
                        Ok(Content::Symlink(MPath::new(data)?))
                    }
                    Type::Tree => {
                        let data = data.as_slice()
                            .ok_or(failure::err_msg("missing tree blob data"))?;
                        let revlog_manifest = RevlogManifest::parse_with_prefix(
                            Some(self.repo.clone()),
                            node.parents(),
                            &data,
                            self.get_path()
                                .mpath()
                                .expect("trees should always have a path"),
                        )?;
                        Ok(Content::Tree(Box::new(revlog_manifest)))
                    }
                }
            })
            .map_err(|err| {
                err.context(format_err!(
                    "Can't get content for {} node {}",
                    self.get_path(),
                    nodeid
                ))
            })
            .map_err(Error::from)
            .into_future()
            .boxify()
    }

    fn get_size(&self) -> BoxFuture<Option<usize>, Error> {
        self.get_content()
            .and_then(|content| match content {
                Content::File(data) | Content::Executable(data) => Ok(data.size()),
                Content::Symlink(path) => Ok(Some(path.to_vec().len())),
                Content::Tree(_) => Ok(None),
            })
            .boxify()
    }

    fn get_hash(&self) -> &EntryId {
        &self.details.entryid
    }

    fn get_name(&self) -> &Option<MPathElement> {
        &self.name
    }
}

fn strip_file_metadata(blob: &Blob) -> Blob {
    match blob {
        &Blob::Dirty(ref bytes) => {
            let (_, off) = file::File::extract_meta(bytes);
            Blob::from(bytes.slice_from(off))
        }
        _ => blob.clone(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use mercurial_types_mocks::nodehash::*;

    #[test]
    fn test_find() {
        assert_eq!(find(b"abc123", &b'b'), Some(1));
        assert_eq!(find(b"abc123", &b'x'), None);
        assert_eq!(find(b"abc123abc", &b'b'), Some(1));
        assert_eq!(find(b"", &b'b'), None);
    }

    #[test]
    fn empty() {
        assert_eq!(
            RevlogManifest::parse(None, &Parents::None, b"").unwrap(),
            RevlogManifest {
                repo: None,
                parents: Parents::None,
                content: ManifestContent {
                    files: BTreeMap::new(),
                },
            }
        );
    }

    #[test]
    fn bad_nonil() {
        match RevlogManifest::parse(None, &Parents::None, b"hello123") {
            Ok(m) => panic!("unexpected manifest {:?}", m),
            Err(e) => println!("got expected error: {}", e),
        }
    }

    #[test]
    fn bad_nohash() {
        match RevlogManifest::parse(None, &Parents::None, b"hello123\0") {
            Ok(m) => panic!("unexpected manifest {:?}", m),
            Err(e) => println!("got expected error: {}", e),
        }
    }

    #[test]
    fn bad_badhash1() {
        match RevlogManifest::parse(None, &Parents::None, b"hello123\0abc123") {
            Ok(m) => panic!("unexpected manifest {:?}", m),
            Err(e) => println!("got expected error: {}", e),
        }
    }

    #[test]
    fn good_one() {
        match RevlogManifest::parse(
            None,
            &Parents::One(THREES_HASH),
            b"hello123\0da39a3ee5e6b4b0d3255bfef95601890afd80709xltZZZ\n",
        ) {
            Ok(m) => {
                assert_eq!(m.parents(), &Parents::One(THREES_HASH));
                let expect = vec![
                    (
                        MPath::new(b"hello123").unwrap(),
                        Details {
                            entryid: EntryId::new(
                                "da39a3ee5e6b4b0d3255bfef95601890afd80709".parse().unwrap(),
                            ),
                            flag: Type::Symlink,
                        },
                    ),
                ];
                assert_eq!(m.content.files.into_iter().collect::<Vec<_>>(), expect);
            }
            Err(e) => println!("got expected error: {}", e),
        }
    }

    #[test]
    fn one_roundtrip() {
        // Only one flag because its unclear how multiple flags should be ordered
        const RAW: &[u8] = b"hello123\0da39a3ee5e6b4b0d3255bfef95601890afd80709x\n";
        let m = RevlogManifest::parse(None, &Parents::None, RAW).expect("failed to parse");

        let mut out = Vec::new();
        m.generate(&mut out).expect("generate failed");

        if RAW != &out[..] {
            println!("\nRAW: {:?}", str::from_utf8(RAW));
            println!("out: {:?}", str::from_utf8(out.as_ref()));
            panic!(
                "out ({} bytes) mismatch RAW ({} bytes)",
                RAW.len(),
                out.len()
            );
        }

        assert_eq!(m.parents(), &Parents::None);
    }

    const MANIFEST: &[u8] = include_bytes!("flatmanifest.bin");

    #[test]
    fn fullmanifest() {
        match RevlogManifest::parse(None, &Parents::Two(ONES_HASH, TWOS_HASH), MANIFEST) {
            Ok(m) => {
                println!("Got manifest:");
                for (k, v) in &m.content.files {
                    println!("{:?} {:?}", k, v);
                }
                assert_eq!(m.parents(), &Parents::Two(ONES_HASH, TWOS_HASH));
            }
            Err(e) => panic!("Failed to load manifest: {}", e),
        }
    }

    #[test]
    fn roundtrip() {
        let m =
            RevlogManifest::parse(None, &Parents::One(ONES_HASH), MANIFEST).expect("parse failed");

        let mut out = Vec::new();
        m.generate(&mut out).expect("generate failed");

        if MANIFEST != &out[..] {
            panic!(
                "out ({} bytes) mismatch MANIFEST ({} bytes)",
                MANIFEST.len(),
                out.len()
            )
        }

        assert_eq!(m.parents(), &Parents::One(ONES_HASH));
    }
}
