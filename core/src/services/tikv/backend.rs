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

use std::collections::HashMap;
use tikv_client::Config;
use tikv_client::RawClient;

use crate::raw::adapters::kv;
use crate::Capability;
use crate::Scheme;
use async_trait::async_trait;
use tokio::sync::OnceCell;

use crate::Builder;
use crate::Error;
use crate::ErrorKind;
use crate::*;

use std::fmt::Debug;
use std::fmt::Formatter;

/// TiKV backend builder
#[derive(Clone, Default)]
pub struct TikvBuilder {
    /// network address of the TiKV service.
    endpoints: Option<Vec<String>>,
    /// whether using insecure connection to TiKV
    insecure: bool,
    /// certificate authority file path
    ca_path: Option<String>,
    /// cert path
    cert_path: Option<String>,
    /// key path
    key_path: Option<String>,
}

impl TikvBuilder {
    /// Set the network address of the TiKV service.
    pub fn endpoints(&mut self, endpoints: Vec<String>) -> &mut Self {
        if !endpoints.is_empty() {
            self.endpoints = Some(endpoints)
        }
        self
    }

    /// Set the insecure connection to TiKV.
    pub fn insecure(&mut self) -> &mut Self {
        self.insecure = true;
        self
    }

    /// Set the certificate authority file path.
    pub fn ca_path(&mut self, ca_path: &str) -> &mut Self {
        if !ca_path.is_empty() {
            self.ca_path = Some(ca_path.to_string())
        }
        self
    }

    /// Set the certificate file path.
    pub fn cert_path(&mut self, cert_path: &str) -> &mut Self {
        if !cert_path.is_empty() {
            self.cert_path = Some(cert_path.to_string())
        }
        self
    }

    /// Set the key file path.
    pub fn key_path(&mut self, key_path: &str) -> &mut Self {
        if !key_path.is_empty() {
            self.key_path = Some(key_path.to_string())
        }
        self
    }
}

impl Builder for TikvBuilder {
    const SCHEME: Scheme = Scheme::Tikv;
    type Accessor = Backend;

    fn from_map(map: HashMap<String, String>) -> Self {
        let mut builder = TikvBuilder::default();

        map.get("endpoints")
            .map(|v| v.split(',').map(|s| s.to_owned()).collect::<Vec<String>>())
            .map(|v| builder.endpoints(v));
        map.get("insecure")
            .filter(|v| *v == "on" || *v == "true")
            .map(|_| builder.insecure());
        map.get("ca_path").map(|v| builder.ca_path(v));
        map.get("cert_path").map(|v| builder.cert_path(v));
        map.get("key_path").map(|v| builder.key_path(v));

        builder
    }

    fn build(&mut self) -> Result<Self::Accessor> {
        let endpoints = self.endpoints.take().ok_or_else(|| {
            Error::new(
                ErrorKind::ConfigInvalid,
                "endpoints is required but not set",
            )
            .with_context("service", Scheme::Tikv)
        })?;

        if self.insecure
            && (self.ca_path.is_some() || self.key_path.is_some() || self.cert_path.is_some())
        {
            return Err(
                Error::new(ErrorKind::ConfigInvalid, "invalid tls configuration")
                    .with_context("service", Scheme::Tikv)
                    .with_context("endpoints", format!("{:?}", endpoints)),
            )?;
        }

        Ok(Backend::new(Adapter {
            client: OnceCell::new(),
            endpoints,
            insecure: self.insecure,
            ca_path: self.ca_path.clone(),
            cert_path: self.cert_path.clone(),
            key_path: self.key_path.clone(),
        }))
    }
}

/// Backend for TiKV service
pub type Backend = kv::Backend<Adapter>;

#[derive(Clone)]
pub struct Adapter {
    client: OnceCell<RawClient>,
    endpoints: Vec<String>,
    insecure: bool,
    ca_path: Option<String>,
    cert_path: Option<String>,
    key_path: Option<String>,
}

impl Debug for Adapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut ds = f.debug_struct("Adapter");
        ds.field("endpoints", &self.endpoints);
        ds.finish()
    }
}

impl Adapter {
    async fn get_connection(&self) -> Result<RawClient> {
        if let Some(client) = self.client.get() {
            return Ok(client.clone());
        }
        let client = if self.insecure {
            RawClient::new(self.endpoints.clone())
                .await
                .map_err(parse_tikv_config_error)?
        } else if self.ca_path.is_some() && self.key_path.is_some() && self.cert_path.is_some() {
            let (ca_path, key_path, cert_path) = (
                self.ca_path.clone().unwrap(),
                self.key_path.clone().unwrap(),
                self.cert_path.clone().unwrap(),
            );
            let config = Config::default().with_security(ca_path, cert_path, key_path);
            RawClient::new_with_config(self.endpoints.clone(), config)
                .await
                .map_err(parse_tikv_config_error)?
        } else {
            return Err(
                Error::new(ErrorKind::ConfigInvalid, "invalid configuration")
                    .with_context("service", Scheme::Tikv)
                    .with_context("endpoints", format!("{:?}", self.endpoints)),
            );
        };
        self.client.set(client.clone()).ok();
        Ok(client)
    }
}

#[async_trait]
impl kv::Adapter for Adapter {
    fn metadata(&self) -> kv::Metadata {
        kv::Metadata::new(
            Scheme::Tikv,
            "TiKV",
            Capability {
                read: true,
                write: true,
                blocking: false,
                ..Default::default()
            },
        )
    }

    async fn get(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.get_connection()
            .await?
            .get(path.to_owned())
            .await
            .map_err(parse_tikv_error)
    }

    async fn set(&self, path: &str, value: &[u8]) -> Result<()> {
        self.get_connection()
            .await?
            .put(path.to_owned(), value.to_vec())
            .await
            .map_err(parse_tikv_error)
    }

    async fn delete(&self, path: &str) -> Result<()> {
        self.get_connection()
            .await?
            .delete(path.to_owned())
            .await
            .map_err(parse_tikv_error)
    }
}

fn parse_tikv_error(e: tikv_client::Error) -> Error {
    Error::new(ErrorKind::Unexpected, "error from tikv").set_source(e)
}

fn parse_tikv_config_error(e: tikv_client::Error) -> Error {
    Error::new(ErrorKind::ConfigInvalid, "invalid configuration")
        .with_context("service", Scheme::Tikv)
        .set_source(e)
}
