// Copyright Materialize, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use crate::ast::{Expr, Raw};
use crate::catalog::{
    CatalogConfig, CatalogDatabase, CatalogError, CatalogItem, CatalogItemType, CatalogRole,
    CatalogSchema, CatalogTypeDetails, SessionCatalog,
};
use crate::func::{Func, MZ_CATALOG_BUILTINS, MZ_INTERNAL_BUILTINS, PG_CATALOG_BUILTINS};
use crate::names::{DatabaseSpecifier, FullName, PartialName};
use crate::plan::StatementDesc;
use chrono::MIN_DATETIME;
use lazy_static::lazy_static;
use mz_build_info::DUMMY_BUILD_INFO;
use mz_dataflow_types::sources::{AwsExternalId, SourceConnector};
use mz_expr::{DummyHumanizer, ExprHumanizer, GlobalId, MirScalarExpr};
use mz_lowertest::*;
use mz_ore::now::{EpochMillis, NOW_ZERO};
use mz_repr::{RelationDesc, RelationType, ScalarType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

lazy_static! {
    static ref DUMMY_CONFIG: CatalogConfig = CatalogConfig {
        start_time: MIN_DATETIME,
        start_instant: Instant::now(),
        nonce: 0,
        cluster_id: Uuid::from_u128(0),
        session_id: Uuid::from_u128(0),
        experimental_mode: false,
        safe_mode: false,
        build_info: &DUMMY_BUILD_INFO,
        aws_external_id: AwsExternalId::NotProvided,
        timestamp_frequency: Duration::from_secs(1),
        now: NOW_ZERO.clone(),
        disable_user_indexes: false,
    };
    static ref RTI: ReflectedTypeInfo = {
        let mut rti = ReflectedTypeInfo::default();
        TestCatalogCommand::add_to_reflected_type_info(&mut rti);
        rti
    };
}

/// A dummy [`CatalogItem`] implementation.
///
/// This implementation is suitable for use in tests that plan queries which are
/// not demanding of the catalog, as many methods are unimplemented.
#[derive(Debug)]
pub enum TestCatalogItem {
    /// Represents some kind of data source. Could be a Source/Table/View.
    BaseTable {
        name: FullName,
        id: GlobalId,
        desc: RelationDesc,
    },
    Func(&'static Func),
}

impl CatalogItem for TestCatalogItem {
    fn name(&self) -> &FullName {
        match &self {
            TestCatalogItem::BaseTable { name, .. } => name,
            _ => unimplemented!(),
        }
    }

    fn id(&self) -> GlobalId {
        match &self {
            TestCatalogItem::BaseTable { id, .. } => *id,
            _ => unimplemented!(),
        }
    }

    fn oid(&self) -> u32 {
        unimplemented!()
    }

    fn desc(&self) -> Result<&RelationDesc, CatalogError> {
        match &self {
            TestCatalogItem::BaseTable { desc, .. } => Ok(desc),
            _ => Err(CatalogError::UnknownItem(format!(
                "{:?} does not have a desc() method",
                self
            ))),
        }
    }

    fn func(&self) -> Result<&'static Func, CatalogError> {
        match &self {
            TestCatalogItem::Func(func) => Ok(func),
            _ => Err(CatalogError::UnknownFunction(format!(
                "{:?} does not have a func() method",
                self
            ))),
        }
    }

    fn source_connector(&self) -> Result<&SourceConnector, CatalogError> {
        unimplemented!()
    }

    fn item_type(&self) -> CatalogItemType {
        match &self {
            TestCatalogItem::BaseTable { .. } => CatalogItemType::View,
            TestCatalogItem::Func(_) => CatalogItemType::Func,
        }
    }

    fn create_sql(&self) -> &str {
        unimplemented!()
    }

    fn uses(&self) -> &[GlobalId] {
        unimplemented!()
    }

    fn used_by(&self) -> &[GlobalId] {
        unimplemented!()
    }

    fn index_details(&self) -> Option<(&[MirScalarExpr], GlobalId)> {
        unimplemented!()
    }

    fn table_details(&self) -> Option<&[Expr<Raw>]> {
        unimplemented!()
    }

    fn type_details(&self) -> Option<&CatalogTypeDetails> {
        unimplemented!()
    }
}

/// A dummy [`SessionCatalog`] implementation.
///
/// This implementation is suitable for use in tests that plan queries which are
/// not demanding of the catalog, as many methods are unimplemented.
#[derive(Debug)]
pub struct TestCatalog {
    /// Contains the Materialize builtin functions.
    funcs: HashMap<&'static str, TestCatalogItem>,
    /// Contains dummy data sources.
    tables: HashMap<String, TestCatalogItem>,
    /// Maps (unique id) -> (name of data source).
    /// Exists to support the `*get_item_by_id` functions.
    id_to_name: HashMap<GlobalId, String>,
}

impl Default for TestCatalog {
    fn default() -> Self {
        let mut funcs = HashMap::new();
        for (name, func) in MZ_INTERNAL_BUILTINS.iter() {
            funcs.insert(*name, TestCatalogItem::Func(func));
        }
        for (name, func) in MZ_CATALOG_BUILTINS.iter() {
            funcs.insert(*name, TestCatalogItem::Func(func));
        }
        for (name, func) in PG_CATALOG_BUILTINS.iter() {
            funcs.insert(*name, TestCatalogItem::Func(func));
        }
        Self {
            funcs,
            tables: HashMap::new(),
            id_to_name: HashMap::new(),
        }
    }
}

impl SessionCatalog for TestCatalog {
    fn active_user(&self) -> &str {
        "dummy"
    }

    fn get_prepared_statement_desc(&self, _: &str) -> Option<&StatementDesc> {
        None
    }

    fn active_database(&self) -> &str {
        "dummy"
    }

    fn resolve_database(&self, _: &str) -> Result<&dyn CatalogDatabase, CatalogError> {
        unimplemented!();
    }

    fn resolve_schema(
        &self,
        _: Option<String>,
        _: &str,
    ) -> Result<&dyn CatalogSchema, CatalogError> {
        unimplemented!();
    }

    fn resolve_role(&self, _: &str) -> Result<&dyn CatalogRole, CatalogError> {
        unimplemented!();
    }

    fn resolve_item(&self, partial_name: &PartialName) -> Result<&dyn CatalogItem, CatalogError> {
        if let Some(result) = self.tables.get(&partial_name.item[..]) {
            return Ok(result);
        }
        Err(CatalogError::UnknownItem(partial_name.item.clone()))
    }

    fn resolve_function(
        &self,
        partial_name: &PartialName,
    ) -> Result<&dyn CatalogItem, CatalogError> {
        if let Some(result) = self.funcs.get(&partial_name.item[..]) {
            return Ok(result);
        }
        Err(CatalogError::UnknownFunction(partial_name.item.clone()))
    }

    fn get_item_by_id(&self, id: &GlobalId) -> &dyn CatalogItem {
        &self.tables[&self.id_to_name[id]]
    }

    fn try_get_item_by_id(&self, _: &GlobalId) -> Option<&dyn CatalogItem> {
        unimplemented!();
    }

    fn get_item_by_oid(&self, _: &u32) -> &dyn CatalogItem {
        unimplemented!();
    }

    fn item_exists(&self, _: &FullName) -> bool {
        false
    }

    fn config(&self) -> &CatalogConfig {
        &DUMMY_CONFIG
    }

    fn now(&self) -> EpochMillis {
        (self.config().now)()
    }
}

impl ExprHumanizer for TestCatalog {
    fn humanize_id(&self, id: GlobalId) -> Option<String> {
        DummyHumanizer.humanize_id(id)
    }

    fn humanize_scalar_type(&self, ty: &ScalarType) -> String {
        DummyHumanizer.humanize_scalar_type(ty)
    }
}

/// Contains the arguments for a command for [TestCatalog].
///
/// See [lowertest] for the command syntax.
#[derive(Debug, Serialize, Deserialize, MzReflect)]
pub enum TestCatalogCommand {
    /// Insert a source into the catalog.
    Defsource {
        source_name: String,
        typ: RelationType,
        #[serde(default)]
        column_names: Vec<String>,
    },
}

impl TestCatalog {
    pub(crate) fn execute_commands(&mut self, spec: &str) -> Result<String, String> {
        let mut stream_iter = tokenize(spec)?.into_iter();
        loop {
            let command: Option<TestCatalogCommand> = deserialize_optional(
                &mut stream_iter,
                "TestCatalogCommand",
                &RTI,
                &mut GenericTestDeserializeContext::default(),
            )?;
            if let Some(command) = command {
                match command {
                    TestCatalogCommand::Defsource {
                        source_name,
                        typ,
                        column_names,
                    } => {
                        assert_eq!(
                            typ.arity(),
                            column_names.len(),
                            "Ensure that there are the right number of column names for source {}",
                            source_name
                        );
                        let id = GlobalId::User(self.tables.len() as u64);
                        self.id_to_name.insert(id, source_name.clone());
                        self.tables.insert(
                            source_name.clone(),
                            TestCatalogItem::BaseTable {
                                name: FullName {
                                    database: DatabaseSpecifier::from(Some(
                                        self.active_database().to_string(),
                                    )),
                                    schema: self.active_user().to_string(),
                                    item: source_name,
                                },
                                id,
                                desc: RelationDesc::new(typ, column_names.into_iter()),
                            },
                        );
                    }
                }
            } else {
                break;
            }
        }
        Ok("ok\n".to_string())
    }
}
