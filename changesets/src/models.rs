// Copyright (c) 2018-present, Facebook, Inc.
// All Rights Reserved.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2 or any later version.

use mercurial_types::{HgChangesetId, RepositoryId};

use schema::{changesets, csparents};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[derive(Queryable)]
pub(crate) struct ChangesetRow {
    // Diesel doesn't support unsigned types.
    // TODO (sid0) T26215455: use a custom type here
    pub id: i64,
    pub repo_id: RepositoryId,
    pub cs_id: HgChangesetId,
    pub gen: i64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[derive(Queryable, Insertable)]
#[table_name = "csparents"]
pub(crate) struct ChangesetParentRow {
    pub cs_id: i64,
    pub parent_id: i64,
    pub seq: i32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[derive(Insertable)]
#[table_name = "changesets"]
pub(crate) struct ChangesetInsertRow {
    pub repo_id: RepositoryId,
    pub cs_id: HgChangesetId,
    pub gen: i64,
}
