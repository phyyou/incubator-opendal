// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::io::SeekFrom;
use std::{io, vec};

use anyhow::Result;
use futures::AsyncSeekExt;
use log::debug;

use crate::*;

pub fn behavior_fuzz_tests(op: &Operator) -> Vec<Trial> {
    async_trials!(op, test_fuzz_issue_2717)
}

/// This fuzz test is to reproduce <https://github.com/apache/incubator-opendal/issues/2717>.
///
/// The simplified cases could be seen as:
///
/// ```
/// FuzzInput {
///     actions: [
///         Seek(
///             End(
///                 -2,
///             ),
///         ),
///         Read {
///             size: 0,
///         },
///     ],
///     data: [
///         0,
///         0,
///     ],
///     range: (
///         1,
///         2,
///     )
///   ]
/// }
/// ```
///
/// Which means:
///
/// - A file with 2 bytes of content.
/// - Open as an range reader of `1..2`.
/// - Seek to `End(-2)` first
///
/// The expected result is seek returns InvalidInput error because the seek position
/// is invalid for given range `1..2`. However, the actual behavior is we seek to `0`
/// and results in a panic.
pub async fn test_fuzz_issue_2717(op: Operator) -> Result<()> {
    let cap = op.info().capability();

    if !(cap.read && cap.write & cap.read_with_range) {
        return Ok(());
    }

    let path = uuid::Uuid::new_v4().to_string();
    debug!("Generate a random file: {}", &path);
    let content = gen_fixed_bytes(2);

    op.write(&path, content.clone())
        .await
        .expect("write must succeed");

    let mut r = op.range_reader(&path, 1..2).await?;

    // Perform a seek
    let result = r.seek(SeekFrom::End(-2)).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);

    Ok(())
}
