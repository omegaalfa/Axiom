//! First-stage semantic snapshot infrastructure.
//!
//! This module deliberately does not resolve types or references yet. It
//! adapts the existing declaration-only `ProjectSymbolIndex` into immutable,
//! revisioned stores with compact in-memory IDs. The current editor navigation
//! remains independent of this module during the initial migration.

use super::{ProjectSymbolIndex, ProjectSymbolKind, Visibility};
use axiom_syntax::{PhpSyntax, SyntaxToken};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

fn text_fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

const SCOPE_COMPACTION_RATIO: usize = 8;
const SCOPE_COMPACTION_MIN_TOTAL: usize = 256;

fn should_compact_scopes(total_scopes: usize, active_scopes: usize) -> bool {
    if total_scopes == active_scopes || total_scopes < SCOPE_COMPACTION_MIN_TOTAL {
        return false;
    }
    if active_scopes == 0 {
        return true;
    }
    total_scopes >= active_scopes.saturating_mul(SCOPE_COMPACTION_RATIO)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ScopeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeKind {
    File,
    Namespace,
    Class,
    Interface,
    Trait,
    Enum,
    Function,
    Method,
    Closure,
    ArrowFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImportKind {
    Class,
    Function,
    Constant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportBinding {
    pub alias: String,
    pub target: String,
    pub kind: ImportKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportTable {
    pub classes: HashMap<String, ImportBinding>,
    pub functions: HashMap<String, ImportBinding>,
    pub constants: HashMap<String, ImportBinding>,
}

impl ImportTable {
    fn insert(&mut self, binding: ImportBinding) {
        let table = match binding.kind {
            ImportKind::Class => &mut self.classes,
            ImportKind::Function => &mut self.functions,
            ImportKind::Constant => &mut self.constants,
        };
        table.insert(binding.alias.clone(), binding);
    }

    pub fn get(&self, kind: ImportKind, alias: &str) -> Option<&ImportBinding> {
        match kind {
            ImportKind::Class => self.classes.get(alias),
            ImportKind::Function => self.functions.get(alias),
            ImportKind::Constant => self.constants.get(alias),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BuiltinType {
    Int,
    String,
    Bool,
    Float,
    Array,
    Object,
    Callable,
    Iterable,
    Mixed,
    Void,
    Never,
    Null,
    False,
    True,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclaredType {
    Named { written: String, resolved: String },
    Builtin(BuiltinType),
    Nullable(Box<DeclaredType>),
    Union(Vec<DeclaredType>),
    Intersection(Vec<DeclaredType>),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableBinding {
    pub name: String,
    pub declaration_span: std::ops::Range<usize>,
    pub declared_type: Option<DeclaredType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub kind: ScopeKind,
    pub owner: Option<SymbolId>,
    pub namespace: String,
    pub class_name: Option<String>,
    pub parent_class: Option<String>,
    /// Interfaces extended by an interface declaration. Classes keep using
    /// `parent_class` because PHP class inheritance is single-parent.
    pub interfaces_extended: Vec<String>,
    /// Traits composed directly by this declaration.
    pub traits_used: Vec<String>,
    pub is_static_method: bool,
    pub imports: ImportTable,
    pub bindings: Vec<VariableBinding>,
    pub file: Option<FileId>,
    pub span: std::ops::Range<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScopeStore {
    pub records: Vec<Scope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct FileId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SymbolId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ReferenceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReferenceRole {
    Type,
    Instantiation,
    FunctionCall,
    MethodCall,
    StaticMethodCall,
    PropertyRead,
    PropertyWrite,
    ClassConstantRead,
    GlobalConstantRead,
    Import,
    ReturnType,
    ParameterType,
    Extends,
    Implements,
    TraitUse,
    Instanceof,
    CatchType,
    Attribute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceTarget {
    Resolved(SymbolId),
    Candidates(Vec<SymbolId>),
    Unresolved,
    Dynamic,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticReference {
    pub id: ReferenceId,
    pub file: FileId,
    pub span: std::ops::Range<usize>,
    pub source_scope: ScopeId,
    pub source_symbol: Option<SymbolId>,
    pub role: ReferenceRole,
    pub target: ReferenceTarget,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ReferenceStore {
    pub records: Vec<SemanticReference>,
    pub references_by_target: HashMap<SymbolId, Vec<ReferenceId>>,
    pub references_by_file: HashMap<FileId, Vec<ReferenceId>>,
    pub ambiguous_references: Vec<ReferenceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceConfidence {
    Exact,
    Ambiguous,
    Partial,
    Deferred,
    Unresolved,
}

/// Identifies which engine produced a location. The same value object can be
/// used later to merge semantic and LSP reference results without coupling the
/// index to UI presentation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceProvider {
    Semantic,
    Lsp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindUsagesOptions {
    pub include_imports: bool,
    pub include_type_references: bool,
}

impl Default for FindUsagesOptions {
    fn default() -> Self {
        Self {
            include_imports: false,
            include_type_references: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceLocation {
    pub file: PersistentFileKey,
    pub span: std::ops::Range<usize>,
    pub role: ReferenceRole,
    pub confidence: ReferenceConfidence,
    pub source_symbol: Option<PersistentSymbolKey>,
    pub provider: ReferenceProvider,
}

pub type UsageLocation = ReferenceLocation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindUsagesStatus {
    Complete,
    Partial,
    Ambiguous,
    Deferred,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindUsagesResult {
    pub usages: Vec<UsageLocation>,
    pub status: FindUsagesStatus,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize,
)]
pub struct SemanticRevision(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceOrigin {
    Workspace,
    Vendor { package: Option<String> },
    Runtime,
    Generated,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersistentFileKey {
    pub origin: SourceOrigin,
    pub normalized_path: String,
}

impl PersistentFileKey {
    pub fn workspace(path: impl AsRef<Path>) -> Self {
        Self::workspace_physical(path)
    }

    /// Lexical Workspace identity for hot paths where filesystem access is not
    /// permitted. Callers must provide a path whose physical identity has
    /// already been stabilized (for example by ProjectSymbolIndex discovery).
    pub fn workspace_lexical(path: impl AsRef<Path>) -> Self {
        Self {
            origin: SourceOrigin::Workspace,
            normalized_path: normalize_path(path.as_ref()),
        }
    }

    /// Canonical identity for Workspace files; falls back lexically for unsaved paths.
    pub fn workspace_physical(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let physical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Self {
            origin: SourceOrigin::Workspace,
            normalized_path: normalize_path(&physical),
        }
    }

    /// Resolves relative paths against an explicit Workspace root.
    pub fn workspace_physical_with_root(path: impl AsRef<Path>, root: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.as_ref().join(path)
        };
        Self::workspace_physical(resolved)
    }

    pub fn new(origin: SourceOrigin, path: impl AsRef<Path>) -> Self {
        Self {
            origin,
            normalized_path: normalize_path(path.as_ref()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersistentSymbolKey {
    pub file: PersistentFileKey,
    pub kind: ProjectSymbolKind,
    pub qualified_name: String,
    /// Disambiguates duplicate declarations with the same FQN in one file.
    /// Unique declarations intentionally carry no order-dependent value.
    pub discriminator: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: FileId,
    pub key: PersistentFileKey,
    pub path: PathBuf,
    pub symbols: Vec<SymbolId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSymbol {
    pub id: SymbolId,
    pub key: PersistentSymbolKey,
    pub name: String,
    pub fully_qualified_name: String,
    pub kind: ProjectSymbolKind,
    pub file: FileId,
    pub range: std::ops::Range<usize>,
    pub namespace: String,
    pub visibility: Visibility,
    pub modifiers: Vec<String>,
    pub parameters: Option<String>,
    pub return_type: Option<String>,
    pub owner: Option<SymbolId>,
    pub owner_key: Option<PersistentSymbolKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileStore {
    pub records: Vec<FileRecord>,
    pub by_key: HashMap<PersistentFileKey, FileId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolStore {
    pub records: Vec<SemanticSymbol>,
    pub by_key: HashMap<PersistentSymbolKey, SymbolId>,
    pub by_fqn: HashMap<String, Vec<SymbolId>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Method,
    Property,
    ClassConstant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberAccess {
    Instance,
    Static,
    /// Class-scope syntax (`self::`, `static::`, `parent::`) may target either
    /// a static or an instance method in PHP. The receiver still determines
    /// the lookup owner; this variant only suppresses the static/instance
    /// incompatibility check while preserving the access context for
    /// visibility checks.
    ClassScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberResolution {
    Resolved(SymbolId),
    Candidates(Vec<SymbolId>),
    ResolvedButInaccessible(SymbolId),
    Incompatible(SymbolId),
    Deferred(String),
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionConfidence {
    Exact,
    High,
    Partial,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionLocation {
    pub file: PathBuf,
    pub span: std::ops::Range<usize>,
    pub origin: SourceOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionCandidate {
    pub location: DefinitionLocation,
    pub confidence: DefinitionConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionResult {
    Resolved(DefinitionCandidate),
    Candidates(Vec<DefinitionCandidate>),
    Deferred(String),
    Unresolved,
}

/// Explain why a semantic definition query did or did not produce a target.
/// This is intentionally a small, local diagnostic vocabulary: callers can
/// keep legacy/LSP fallbacks while still measuring which semantic gap caused
/// the fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDefinitionOutcome {
    Resolved,
    Ambiguous,
    DeferredVendor,
    StaleSnapshot,
    UnsupportedSyntax,
    UnknownReceiverType,
    MissingSymbol,
    Inaccessible,
    IncompatibleAccess,
    IncompleteAst,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InaccessibilityReason {
    PrivateMember,
    ProtectedMember,
    IncompatibleAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InaccessibilityInfo {
    pub reason: InaccessibilityReason,
    pub declaring_type: String,
    pub member_name: String,
    pub access_context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDefinitionResult {
    pub result: DefinitionResult,
    pub outcome: SemanticDefinitionOutcome,
}

#[derive(Debug, Clone)]
pub struct DefinitionSyntaxContext {
    pub token: SyntaxToken,
    pub is_keyword: bool,
    pub tree: tree_sitter::Tree,
    pub build_us: u128,
}

#[derive(Debug, Clone, Copy)]
pub struct DefinitionQueryContext {
    pub document_version: Option<u64>,
    pub semantic_revision: SemanticRevision,
}

pub struct MemberResolver<'a> {
    snapshot: &'a SemanticSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Variable(String),
    New(String),
    FunctionCall(String),
    MethodCall {
        receiver: Box<Expression>,
        name: String,
        access: MemberAccess,
    },
    Parenthesized(Box<Expression>),
    Special(String),
}

pub struct ExpressionResolver<'a> {
    snapshot: &'a SemanticSnapshot,
    scope: ScopeId,
}

impl<'a> ExpressionResolver<'a> {
    pub fn new(snapshot: &'a SemanticSnapshot, scope: ScopeId) -> Self {
        Self { snapshot, scope }
    }

    pub fn infer_expression_type(&self, expression: &Expression) -> Option<DeclaredType> {
        match expression {
            Expression::Variable(name) => self
                .snapshot
                .lookup_binding(self.scope, name)
                .and_then(|binding| binding.declared_type.clone()),
            Expression::New(name) => {
                self.snapshot
                    .resolve_class_name(self.scope, name)
                    .map(|resolved| DeclaredType::Named {
                        written: name.clone(),
                        resolved,
                    })
            }
            Expression::Special(name) => {
                self.snapshot
                    .resolve_class_name(self.scope, name)
                    .map(|resolved| DeclaredType::Named {
                        written: name.clone(),
                        resolved,
                    })
            }
            Expression::Parenthesized(inner) => self.infer_expression_type(inner),
            Expression::FunctionCall(name) => self.function_return_type(name),
            Expression::MethodCall {
                receiver,
                name,
                access,
            } => {
                let receiver_type = self.infer_expression_type(receiver)?;
                let resolution = self.snapshot.member_resolver().resolve_method(
                    self.scope,
                    &receiver_type,
                    name,
                    *access,
                );
                let MemberResolution::Resolved(id) = resolution else {
                    return None;
                };
                self.return_type_of(id)
            }
        }
    }

    pub fn resolve_member_chain(
        &self,
        receiver: &Expression,
        name: &str,
        access: MemberAccess,
    ) -> MemberResolution {
        let Some(receiver_type) = self.infer_expression_type(receiver) else {
            return MemberResolution::Unresolved;
        };
        self.snapshot
            .member_resolver()
            .resolve_method(self.scope, &receiver_type, name, access)
    }

    fn function_return_type(&self, name: &str) -> Option<DeclaredType> {
        let resolved = self.snapshot.resolve_function_name(self.scope, name)?;
        let symbol = self
            .snapshot
            .symbols_for_fqn(&resolved)
            .iter()
            .filter_map(|id| self.snapshot.symbol(*id))
            .find(|symbol| symbol.kind == ProjectSymbolKind::Function)?;
        self.return_type_of(symbol.id)
    }

    fn return_type_of(&self, id: SymbolId) -> Option<DeclaredType> {
        let symbol = self.snapshot.symbol(id)?;
        let raw = symbol.return_type.as_deref()?.trim();
        if raw.is_empty() {
            return None;
        }
        Some(declared_type_from_snapshot(self.snapshot, self.scope, raw))
    }
}

fn declared_type_from_snapshot(
    snapshot: &SemanticSnapshot,
    scope: ScopeId,
    raw: &str,
) -> DeclaredType {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('?') {
        return DeclaredType::Nullable(Box::new(declared_type_from_snapshot(
            snapshot, scope, inner,
        )));
    }
    if raw.contains('|') {
        return DeclaredType::Union(
            raw.split('|')
                .map(|part| declared_type_from_snapshot(snapshot, scope, part))
                .collect(),
        );
    }
    if raw.contains('&') {
        return DeclaredType::Intersection(
            raw.split('&')
                .map(|part| declared_type_from_snapshot(snapshot, scope, part))
                .collect(),
        );
    }
    let builtin = match raw.to_ascii_lowercase().as_str() {
        "int" => Some(BuiltinType::Int),
        "string" => Some(BuiltinType::String),
        "bool" => Some(BuiltinType::Bool),
        "float" => Some(BuiltinType::Float),
        "array" => Some(BuiltinType::Array),
        "object" => Some(BuiltinType::Object),
        "callable" => Some(BuiltinType::Callable),
        "iterable" => Some(BuiltinType::Iterable),
        "mixed" => Some(BuiltinType::Mixed),
        "void" => Some(BuiltinType::Void),
        "never" => Some(BuiltinType::Never),
        "null" => Some(BuiltinType::Null),
        "false" => Some(BuiltinType::False),
        "true" => Some(BuiltinType::True),
        _ => None,
    };
    if let Some(builtin) = builtin {
        return DeclaredType::Builtin(builtin);
    }
    let resolved = snapshot
        .resolve_class_name(scope, raw)
        .unwrap_or_else(|| raw.to_owned());
    DeclaredType::Named {
        written: raw.to_owned(),
        resolved,
    }
}

fn expression_from_ast(node: tree_sitter::Node<'_>, text: &str) -> Option<Expression> {
    match node.kind() {
        "variable_name" => Some(Expression::Variable(
            node_text(node, text).trim().to_owned(),
        )),
        "object_creation_expression" => node
            .child_by_field_name("class")
            .map(|class| Expression::New(node_text(class, text).trim().to_owned())),
        "parenthesized_expression" => node
            .named_children(&mut node.walk())
            .next()
            .and_then(|inner| expression_from_ast(inner, text))
            .map(|inner| Expression::Parenthesized(Box::new(inner))),
        "member_call_expression" | "nullsafe_member_call_expression" => {
            let object = node.child_by_field_name("object")?;
            let name = node.child_by_field_name("name")?;
            Some(Expression::MethodCall {
                receiver: Box::new(expression_from_ast(object, text)?),
                name: node_text(name, text).trim().to_owned(),
                access: MemberAccess::Instance,
            })
        }
        "static_call_expression" => {
            let class = node.child_by_field_name("class")?;
            let name = node.child_by_field_name("name")?;
            Some(Expression::MethodCall {
                receiver: Box::new(Expression::Special(
                    node_text(class, text).trim().to_owned(),
                )),
                name: node_text(name, text).trim().to_owned(),
                access: MemberAccess::Static,
            })
        }
        "function_call_expression" => node
            .child_by_field_name("function")
            .map(|function| Expression::FunctionCall(node_text(function, text).trim().to_owned())),
        _ => None,
    }
}

impl<'a> MemberResolver<'a> {
    pub fn new(snapshot: &'a SemanticSnapshot) -> Self {
        Self { snapshot }
    }

    pub fn resolve_method(
        &self,
        scope: ScopeId,
        receiver: &DeclaredType,
        name: &str,
        access: MemberAccess,
    ) -> MemberResolution {
        self.resolve(scope, receiver, name, MemberKind::Method, access)
    }

    pub fn resolve_property(
        &self,
        scope: ScopeId,
        receiver: &DeclaredType,
        name: &str,
    ) -> MemberResolution {
        self.resolve(
            scope,
            receiver,
            name,
            MemberKind::Property,
            MemberAccess::Instance,
        )
    }

    pub fn resolve_class_constant(
        &self,
        scope: ScopeId,
        receiver: &DeclaredType,
        name: &str,
    ) -> MemberResolution {
        self.resolve(
            scope,
            receiver,
            name,
            MemberKind::ClassConstant,
            MemberAccess::Static,
        )
    }

    pub fn resolve_binding_method(
        &self,
        scope: ScopeId,
        binding: &str,
        name: &str,
    ) -> MemberResolution {
        let Some(binding) = self.snapshot.lookup_binding(scope, binding) else {
            return MemberResolution::Unresolved;
        };
        let Some(declared_type) = binding.declared_type.as_ref() else {
            return MemberResolution::Unresolved;
        };
        self.resolve_method(scope, declared_type, name, MemberAccess::Instance)
    }

    /// Returns the instance methods visible on a typed binding, including
    /// methods inherited through classes and composed through traits.
    ///
    /// The result is derived exclusively from the immutable snapshot. No
    /// parsing, filesystem access, or project-wide index scan is performed;
    /// traversal is limited to the owners reachable from the binding type.
    pub fn completion_methods_for_binding(
        &self,
        scope: ScopeId,
        binding_name: &str,
        prefix: &str,
    ) -> Vec<SymbolId> {
        let Some(binding) = self.snapshot.lookup_binding(scope, binding_name) else {
            return Vec::new();
        };
        let Some(DeclaredType::Named { resolved, .. }) = binding.declared_type.as_ref() else {
            return Vec::new();
        };

        let prefix = prefix.trim();
        let mut result = Vec::new();
        let mut seen_names = HashSet::new();
        for (owner, _, _) in self.inheritance_chain_with_depth(resolved) {
            let mut owner_ids = self.snapshot.symbols_for_fqn(&owner).to_vec();
            owner_ids.sort_by_key(|id| id.0);
            for owner_id in owner_ids {
                let mut methods = self
                    .snapshot
                    .members_of(owner_id)
                    .iter()
                    .filter_map(|id| self.snapshot.symbol(*id).map(|symbol| (*id, symbol)))
                    .filter(|(_, symbol)| {
                        symbol.kind == ProjectSymbolKind::Method
                            && symbol.name.starts_with(prefix)
                            && !symbol.modifiers.iter().any(|modifier| modifier == "static")
                            && self.is_accessible(scope, symbol)
                    })
                    .collect::<Vec<_>>();
                methods.sort_by_key(|(id, _)| id.0);
                for (id, symbol) in methods {
                    // The first declaration in the owner traversal wins, so
                    // child overrides do not produce a second completion item.
                    if seen_names.insert(symbol.name.clone()) {
                        result.push(id);
                    }
                }
            }
        }
        result
    }

    pub fn resolve_special_method(
        &self,
        scope: ScopeId,
        receiver: &str,
        name: &str,
        access: MemberAccess,
    ) -> MemberResolution {
        let Some(class_name) = self.snapshot.resolve_class_name(scope, receiver) else {
            return MemberResolution::Unresolved;
        };
        let declared_type = DeclaredType::Named {
            written: receiver.to_owned(),
            resolved: class_name,
        };
        self.resolve_method(scope, &declared_type, name, access)
    }

    fn resolve(
        &self,
        scope: ScopeId,
        receiver: &DeclaredType,
        name: &str,
        kind: MemberKind,
        access: MemberAccess,
    ) -> MemberResolution {
        let DeclaredType::Named { resolved, .. } = receiver else {
            return MemberResolution::Unresolved;
        };
        let project_kind = match kind {
            MemberKind::Method => ProjectSymbolKind::Method,
            MemberKind::Property => ProjectSymbolKind::Property,
            MemberKind::ClassConstant => ProjectSymbolKind::ClassConstant,
        };
        let owners = self.inheritance_chain_with_depth(resolved);
        if owners.is_empty() {
            // The nominal type can be known before a lazy Composer/vendor file
            // has entered the semantic snapshot.
            return MemberResolution::Deferred(format!("members for {resolved} are not indexed"));
        }
        // PHP member lookup observes the first declaration in the inheritance
        // chain. A child declaration therefore shadows every parent member,
        // even when the declaration is later rejected as inaccessible or has
        // the wrong static/instance form.
        let mut candidates = Vec::new();
        let mut found_depth = None;
        let mut found_rank = None;
        for (owner, depth, rank) in owners {
            if found_depth.is_some_and(|found| (depth, rank) > (found, found_rank.unwrap_or(0))) {
                break;
            }
            let mut declared = self
                .snapshot
                .symbols_for_fqn(&owner)
                .iter()
                .filter(|id| {
                    self.snapshot.symbol(**id).is_some_and(|symbol| {
                        matches!(
                            symbol.kind,
                            ProjectSymbolKind::Class
                                | ProjectSymbolKind::Interface
                                | ProjectSymbolKind::Trait
                                | ProjectSymbolKind::Enum
                        )
                    })
                })
                .flat_map(|id| {
                    let mut matches = self
                        .snapshot
                        .members_named(*id, name, project_kind)
                        .to_vec();
                    if kind == MemberKind::ClassConstant {
                        matches.extend(
                            self.snapshot
                                .members_named(*id, name, ProjectSymbolKind::Constant)
                                .iter()
                                .copied(),
                        );
                        matches.extend(
                            self.snapshot
                                .members_named(*id, name, ProjectSymbolKind::EnumCase)
                                .iter()
                                .copied(),
                        );
                    }
                    matches
                })
                .collect::<Vec<_>>();
            declared.sort_by_key(|id| id.0);
            declared.dedup();
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                let files = self
                    .snapshot
                    .symbols_for_fqn(&owner)
                    .iter()
                    .filter_map(|id| self.snapshot.symbol(*id))
                    .map(|symbol| symbol.file.0.to_string())
                    .collect::<Vec<_>>();
                eprintln!(
                    "[INHERIT TRACE] receiver={} lookup={} owner={} members={:?} files={:?}",
                    resolved, name, owner, declared, files
                );
            }
            if !declared.is_empty() {
                candidates.extend(declared);
                if found_depth.is_none()
                    || (depth, rank) < (found_depth.unwrap(), found_rank.unwrap_or(rank))
                {
                    found_depth = Some(depth);
                    found_rank = Some(rank);
                }
            }
        }
        candidates.sort_by_key(|id| id.0);
        candidates.dedup();
        if candidates.is_empty() {
            return MemberResolution::Unresolved;
        }
        let mut valid = Vec::new();
        let mut inaccessible = Vec::new();
        let mut incompatible = Vec::new();
        for id in candidates {
            let Some(symbol) = self.snapshot.symbol(id) else {
                continue;
            };
            if kind == MemberKind::Method {
                let is_static = symbol.modifiers.iter().any(|modifier| modifier == "static");
                if !matches!(access, MemberAccess::ClassScope)
                    && is_static != matches!(access, MemberAccess::Static)
                {
                    incompatible.push(id);
                    continue;
                }
            }
            if !self.is_accessible(scope, symbol) {
                inaccessible.push(id);
            } else {
                valid.push(id);
            }
        }
        if valid.len() == 1 {
            MemberResolution::Resolved(valid[0])
        } else if valid.len() > 1 {
            MemberResolution::Candidates(valid)
        } else if let Some(id) = inaccessible.first().copied() {
            MemberResolution::ResolvedButInaccessible(id)
        } else if let Some(id) = incompatible.first().copied() {
            MemberResolution::Incompatible(id)
        } else {
            MemberResolution::Unresolved
        }
    }

    fn is_accessible(&self, scope: ScopeId, symbol: &SemanticSymbol) -> bool {
        if symbol.kind == ProjectSymbolKind::EnumCase {
            return true;
        }
        let visibility = symbol.visibility;
        if visibility == Visibility::Public {
            return true;
        }
        let Some(owner) = symbol.owner else {
            return false;
        };
        let Some(owner_symbol) = self.snapshot.symbol(owner) else {
            return false;
        };
        let owner_name = owner_symbol
            .fully_qualified_name
            .split_once("::")
            .map(|(owner, _)| owner)
            .unwrap_or(owner_symbol.fully_qualified_name.as_str());
        let mut current = Some(scope);
        while let Some(id) = current {
            let Some(scope) = self.snapshot.scope(id) else {
                break;
            };
            if let Some(class_name) = scope.class_name.as_deref() {
                if visibility == Visibility::Private {
                    if class_name == owner_name {
                        return true;
                    }
                } else if self.is_same_or_subclass(class_name, owner_name) {
                    return true;
                }
            }
            current = scope.parent;
        }
        false
    }

    fn inheritance_chain(&self, root: &str) -> Vec<String> {
        self.inheritance_chain_with_depth(root)
            .into_iter()
            .map(|(name, _, _)| name)
            .collect()
    }

    fn inheritance_chain_with_depth(&self, root: &str) -> Vec<(String, usize, u8)> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = vec![(root.to_owned(), 0usize, 0u8)];
        while let Some((current, depth, rank)) = queue.first().cloned() {
            queue.remove(0);
            if !visited.insert(current.clone()) {
                continue;
            }
            let owner_ids: Vec<_> = self
                .snapshot
                .symbols_for_fqn(&current)
                .iter()
                .copied()
                .collect();
            let has_owner = owner_ids.iter().any(|id| {
                self.snapshot.symbol(*id).is_some_and(|symbol| {
                    matches!(
                        symbol.kind,
                        ProjectSymbolKind::Class
                            | ProjectSymbolKind::Interface
                            | ProjectSymbolKind::Trait
                            | ProjectSymbolKind::Enum
                    )
                })
            });
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                for id in &owner_ids {
                    if let Some(symbol) = self.snapshot.symbol(*id) {
                        eprintln!(
                            "[INHERIT TRACE] receiver={root} owner={current} symbol_id={:?} file={} kind={:?}",
                            id, symbol.file.0, symbol.kind
                        );
                    }
                }
            }
            if !has_owner {
                break;
            }
            chain.push((current.clone(), depth, rank));
            let traits = self.traits_of(&current);
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() && !traits.is_empty() {
                eprintln!("[INHERIT TRACE] owner={current} traits_used={traits:?}");
            }
            for trait_name in traits {
                queue.push((trait_name, depth + 1, 1));
            }
            let parents = self.parents_of(&current);
            for parent in parents {
                if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                    eprintln!(
                        "[INHERIT TRACE] receiver={root} owner={current} parent_fqn={parent}"
                    );
                }
                queue.push((parent, depth + 1, 2));
            }
        }
        chain
    }

    fn parents_of(&self, class_name: &str) -> Vec<String> {
        let Some(scope) = self.snapshot.scopes.records.iter().find(|scope| {
            matches!(
                scope.kind,
                ScopeKind::Class | ScopeKind::Interface | ScopeKind::Enum
            ) && scope.class_name.as_deref() == Some(class_name)
        }) else {
            return Vec::new();
        };
        if scope.kind == ScopeKind::Interface {
            scope.interfaces_extended.clone()
        } else {
            self.parent_class_of(class_name).into_iter().collect()
        }
    }

    fn traits_of(&self, class_name: &str) -> Vec<String> {
        let mut names = self
            .snapshot
            .scopes
            .records
            .iter()
            .find(|scope| {
                scope.file.is_some()
                    && matches!(scope.kind, ScopeKind::Class | ScopeKind::Trait)
                    && scope.class_name.as_deref() == Some(class_name)
            })
            .map(|scope| scope.traits_used.clone())
            .unwrap_or_default();
        if names.is_empty() {
            let class_scope = self.snapshot.scopes.records.iter().find(|scope| {
                scope.file.is_some() && scope.class_name.as_deref() == Some(class_name)
            });
            if let Some(owner) = self
                .snapshot
                .symbols_for_fqn(class_name)
                .iter()
                .copied()
                .find(|id| {
                    self.snapshot.symbol(*id).is_some_and(|s| {
                        s.kind == ProjectSymbolKind::Class || s.kind == ProjectSymbolKind::Trait
                    })
                })
            {
                for reference in &self.snapshot.references.records {
                    if (reference.source_symbol == Some(owner)
                        || class_scope.is_some_and(|scope| reference.source_scope == scope.id))
                        && reference.role == ReferenceRole::TraitUse
                        && let ReferenceTarget::Resolved(id) = reference.target
                        && let Some(symbol) = self.snapshot.symbol(id)
                    {
                        names.push(symbol.fully_qualified_name.clone());
                    }
                }
            }
        }
        names
    }

    fn parent_class_of(&self, class_name: &str) -> Option<String> {
        self.snapshot
            .scopes
            .records
            .iter()
            .find(|scope| {
                matches!(
                    scope.kind,
                    ScopeKind::Class | ScopeKind::Interface | ScopeKind::Enum
                ) && scope.class_name.as_deref() == Some(class_name)
            })
            .and_then(|scope| scope.parent_class.clone())
    }

    fn is_same_or_subclass(&self, candidate: &str, ancestor: &str) -> bool {
        self.inheritance_chain(candidate)
            .iter()
            .any(|name| name == ancestor)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeclarationIndexes {
    pub symbols_by_file: HashMap<FileId, Vec<SymbolId>>,
    pub members_by_owner: HashMap<SymbolId, Vec<SymbolId>>,
    pub members_by_owner_name: HashMap<(SymbolId, String, ProjectSymbolKind), Vec<SymbolId>>,
}

/// Derived semantic relationships between interfaces and classes. These are
/// rebuilt from the immutable symbol/reference graph whenever a snapshot is
/// published, including incremental file replacements.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterfaceRelationIndexes {
    direct_implementers_by_interface: HashMap<SymbolId, Vec<SymbolId>>,
    implementers_by_interface: HashMap<SymbolId, Vec<SymbolId>>,
    method_implementations_by_interface_method: HashMap<SymbolId, Vec<SymbolId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSnapshot {
    pub revision: SemanticRevision,
    files: FileStore,
    symbols: SymbolStore,
    declarations: DeclarationIndexes,
    references: ReferenceStore,
    pub scopes: ScopeStore,
    pub file_fingerprints: HashMap<FileId, u64>,
    pub interface_relations: InterfaceRelationIndexes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSnapshot {
    revision: SemanticRevision,
    files: Vec<FileRecord>,
    symbols: Vec<SemanticSymbol>,
    #[serde(default)]
    references: ReferenceStore,
    #[serde(default)]
    scopes: ScopeStore,
    #[serde(default)]
    file_fingerprints: HashMap<FileId, u64>,
}

impl Default for SemanticSnapshot {
    fn default() -> Self {
        Self::empty(SemanticRevision(0))
    }
}

impl SemanticSnapshot {
    pub fn empty(revision: SemanticRevision) -> Self {
        Self {
            revision,
            files: FileStore::default(),
            symbols: SymbolStore::default(),
            declarations: DeclarationIndexes::default(),
            references: ReferenceStore::default(),
            scopes: ScopeStore::default(),
            file_fingerprints: HashMap::new(),
            interface_relations: InterfaceRelationIndexes::default(),
        }
    }

    pub fn from_project_index(index: &ProjectSymbolIndex, revision: SemanticRevision) -> Self {
        SnapshotBuilder::from_project_index(index, revision).build()
    }

    pub fn file_id(&self, key: &PersistentFileKey) -> Option<FileId> {
        self.files.by_key.get(key).copied()
    }

    /// Finds the lexical scope containing a byte offset in a snapshot file.
    /// The file key must already be present in the snapshot; this method never
    /// canonicalizes, parses, or touches the filesystem.
    pub fn scope_id_at(&self, file: &PersistentFileKey, offset: usize) -> Option<ScopeId> {
        let file_id = self.file_id(file)?;
        self.scope_at(file_id, offset).map(|scope| scope.id)
    }

    /// Variant for callers that already hold the resident compact `FileId`.
    pub fn scope_id_at_file(&self, file: FileId, offset: usize) -> Option<ScopeId> {
        self.scope_at(file, offset).map(|scope| scope.id)
    }

    pub fn symbol_id(&self, key: &PersistentSymbolKey) -> Option<SymbolId> {
        self.symbols.by_key.get(key).copied()
    }

    pub fn symbol(&self, id: SymbolId) -> Option<&SemanticSymbol> {
        self.symbols.records.get(id.0 as usize)
    }

    pub fn file(&self, id: FileId) -> Option<&FileRecord> {
        self.files.records.get(id.0 as usize)
    }

    pub fn symbols_for_fqn(&self, fqn: &str) -> &[SymbolId] {
        self.symbols
            .by_fqn
            .get(fqn)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn symbols_for_file(&self, file: FileId) -> &[SymbolId] {
        self.declarations
            .symbols_by_file
            .get(&file)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn references_for_file(&self, file: FileId) -> &[ReferenceId] {
        self.references
            .references_by_file
            .get(&file)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn references_for_target(&self, target: SymbolId) -> &[ReferenceId] {
        self.references
            .references_by_target
            .get(&target)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn reference(&self, id: ReferenceId) -> Option<&SemanticReference> {
        self.references.records.get(id.0 as usize)
    }

    fn scope_at(&self, file: FileId, offset: usize) -> Option<&Scope> {
        let mut scope = self
            .scopes
            .records
            .iter()
            .filter(|scope| scope.file == Some(file) && scope.span.contains(&offset))
            .max_by_key(|scope| scope.span.start);
        if scope.is_none() {
            // An editor cursor may be ahead of a stale snapshot's last byte.
            // Keep the lookup conservative: return only the resident File
            // scope for this FileId, without extending spans or creating data.
            scope = self.scopes.records.iter().find(|candidate| {
                candidate.file == Some(file)
                    && candidate.kind == ScopeKind::File
                    && offset >= candidate.span.end
            });
        }
        let mut scope = scope?;
        if scope.kind == ScopeKind::File {
            if let Some(namespace) = self
                .scopes
                .records
                .iter()
                .filter(|candidate| {
                    candidate.file == Some(file)
                        && candidate.kind == ScopeKind::Namespace
                        && candidate.span.start <= offset
                })
                .max_by_key(|candidate| candidate.span.start)
            {
                scope = namespace;
            }
        }
        Some(scope)
    }

    pub fn members_of(&self, owner: SymbolId) -> &[SymbolId] {
        self.declarations
            .members_by_owner
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn members_named(
        &self,
        owner: SymbolId,
        name: &str,
        kind: ProjectSymbolKind,
    ) -> &[SymbolId] {
        self.declarations
            .members_by_owner_name
            .get(&(owner, name.trim_start_matches('$').to_owned(), kind))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Classes that explicitly name `interface` in an implements clause.
    pub fn direct_implementers_of(&self, interface: SymbolId) -> &[SymbolId] {
        self.interface_relations
            .direct_implementers_by_interface
            .get(&interface)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// All classes implementing an interface, including implementations
    /// inherited through parent interfaces and parent classes.
    pub fn implementers_of(&self, interface: SymbolId) -> &[SymbolId] {
        self.interface_relations
            .implementers_by_interface
            .get(&interface)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Methods satisfying an interface method contract. The returned symbols
    /// retain their concrete declaring owner (including an inherited method).
    pub fn implementations_of(&self, interface_method: SymbolId) -> &[SymbolId] {
        self.interface_relations
            .method_implementations_by_interface_method
            .get(&interface_method)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn rebuild_interface_relations(&mut self) {
        let active: HashSet<SymbolId> = self
            .files
            .records
            .iter()
            .flat_map(|file| file.symbols.iter().copied())
            .collect();
        let mut extends: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        let mut implements: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        for reference in &self.references.records {
            let target = match reference.target {
                ReferenceTarget::Resolved(id) if active.contains(&id) => id,
                _ => continue,
            };
            let Some(source) = self.owner_for_scope(reference.source_scope, &active) else {
                continue;
            };
            match reference.role {
                ReferenceRole::Extends => extends.entry(source).or_default().push(target),
                ReferenceRole::Implements => implements.entry(source).or_default().push(target),
                _ => {}
            }
        }
        for values in extends.values_mut().chain(implements.values_mut()) {
            values.sort_by_key(|id| id.0);
            values.dedup();
        }

        let mut relations = InterfaceRelationIndexes::default();
        for (&class, interfaces) in &implements {
            if self.symbol(class).is_none_or(|symbol| {
                !matches!(
                    symbol.kind,
                    ProjectSymbolKind::Class | ProjectSymbolKind::Enum
                )
            }) {
                continue;
            }
            for &interface in interfaces {
                if self
                    .symbol(interface)
                    .is_some_and(|symbol| symbol.kind == ProjectSymbolKind::Interface)
                {
                    relations
                        .direct_implementers_by_interface
                        .entry(interface)
                        .or_default()
                        .push(class);
                }
            }
        }

        // A class inherits contracts from its parent classes. An interface
        // also inherits all contracts from parent interfaces.
        let classes: Vec<_> = active
            .iter()
            .copied()
            .filter(|id| {
                self.symbol(*id).is_some_and(|s| {
                    matches!(s.kind, ProjectSymbolKind::Class | ProjectSymbolKind::Enum)
                })
            })
            .collect();
        for class in classes {
            let mut contracts = HashSet::new();
            let mut pending = vec![class];
            let mut visited = HashSet::new();
            while let Some(owner) = pending.pop() {
                if !visited.insert(owner) {
                    continue;
                }
                if let Some(names) = implements.get(&owner) {
                    for &interface in names {
                        collect_interface_ancestors(self, &extends, interface, &mut contracts);
                    }
                }
                if let Some(parents) = extends.get(&owner) {
                    pending.extend(parents.iter().copied().filter(|id| {
                        self.symbol(*id)
                            .is_some_and(|s| s.kind == ProjectSymbolKind::Class)
                    }));
                }
            }
            for interface in contracts {
                relations
                    .implementers_by_interface
                    .entry(interface)
                    .or_default()
                    .push(class);
            }
        }

        for values in relations
            .direct_implementers_by_interface
            .values_mut()
            .chain(relations.implementers_by_interface.values_mut())
        {
            values.sort_by(|left, right| {
                self.symbol(*left)
                    .map(|s| s.fully_qualified_name.as_str())
                    .cmp(&self.symbol(*right).map(|s| s.fully_qualified_name.as_str()))
                    .then(left.0.cmp(&right.0))
            });
            values.dedup();
        }

        // Match each interface method against the effective method on every
        // implementing class, following the same first-owner/override order
        // as MemberResolver.
        let interfaces: Vec<_> = active
            .iter()
            .copied()
            .filter(|id| {
                self.symbol(*id)
                    .is_some_and(|s| s.kind == ProjectSymbolKind::Interface)
            })
            .collect();
        for interface in interfaces {
            let implementers = relations
                .implementers_by_interface
                .get(&interface)
                .cloned()
                .unwrap_or_default();
            for method in self.members_of(interface).iter().copied().filter(|id| {
                self.symbol(*id)
                    .is_some_and(|symbol| symbol.kind == ProjectSymbolKind::Method)
            }) {
                let Some(method_name) = self.symbol(method).map(|symbol| symbol.name.clone())
                else {
                    continue;
                };
                let mut matches = Vec::new();
                for class in &implementers {
                    let mut owners = vec![*class];
                    let mut visited = HashSet::new();
                    let mut found = Vec::new();
                    while let Some(owner) = owners.pop() {
                        if !visited.insert(owner) {
                            continue;
                        }
                        let declared = self
                            .members_named(owner, &method_name, ProjectSymbolKind::Method)
                            .to_vec();
                        if !declared.is_empty() {
                            found = declared;
                            break;
                        }
                        if let Some(parents) = extends.get(&owner) {
                            owners.extend(parents.iter().copied().filter(|id| {
                                self.symbol(*id)
                                    .is_some_and(|s| s.kind == ProjectSymbolKind::Class)
                            }));
                        }
                    }
                    matches.extend(found);
                }
                matches.sort_by(|left, right| {
                    self.symbol(*left)
                        .map(|s| s.fully_qualified_name.as_str())
                        .cmp(&self.symbol(*right).map(|s| s.fully_qualified_name.as_str()))
                        .then(left.0.cmp(&right.0))
                });
                matches.dedup();
                if !matches.is_empty() {
                    relations
                        .method_implementations_by_interface_method
                        .insert(method, matches);
                }
            }
        }
        self.interface_relations = relations;
    }

    fn owner_for_scope(&self, scope_id: ScopeId, active: &HashSet<SymbolId>) -> Option<SymbolId> {
        let scope = self.scope(scope_id)?;
        let name = scope.class_name.as_deref()?;
        self.symbols_for_fqn(name).iter().copied().find(|id| {
            active.contains(id)
                && self.symbol(*id).is_some_and(|s| {
                    matches!(
                        s.kind,
                        ProjectSymbolKind::Class
                            | ProjectSymbolKind::Interface
                            | ProjectSymbolKind::Enum
                    )
                })
        })
    }

    pub fn member_resolver(&self) -> MemberResolver<'_> {
        MemberResolver::new(self)
    }

    pub fn definition_at(
        &self,
        file: impl AsRef<Path>,
        text: &str,
        offset: usize,
        context: DefinitionQueryContext,
    ) -> DefinitionResult {
        self.definition_at_detailed(file, text, offset, context)
            .result
    }

    pub fn definition_at_detailed(
        &self,
        file: impl AsRef<Path>,
        text: &str,
        offset: usize,
        context: DefinitionQueryContext,
    ) -> SemanticDefinitionResult {
        self.definition_at_detailed_with_syntax(file, text, offset, None, context)
    }

    pub fn definition_at_detailed_with_syntax(
        &self,
        file: impl AsRef<Path>,
        text: &str,
        offset: usize,
        syntax_context: Option<DefinitionSyntaxContext>,
        context: DefinitionQueryContext,
    ) -> SemanticDefinitionResult {
        let profile = std::env::var_os("AXIOM_DEBUG_DEFINITION_PROFILE").is_some();
        let profile_started = std::time::Instant::now();
        let revision_check_us;
        let fingerprint_us;
        let scope_lookup_us;
        let text_clone_us;
        let parse_us;
        let mut token_lookup_us;
        let resolution_us;
        let phase = std::time::Instant::now();
        if context.semantic_revision != self.revision {
            return SemanticDefinitionResult {
                result: DefinitionResult::Unresolved,
                outcome: SemanticDefinitionOutcome::StaleSnapshot,
            };
        }
        revision_check_us = phase.elapsed().as_micros();
        let key = PersistentFileKey::workspace(file.as_ref());
        let Some(file_id) = self.file_id(&key) else {
            return SemanticDefinitionResult {
                result: DefinitionResult::Unresolved,
                outcome: SemanticDefinitionOutcome::MissingSymbol,
            };
        };
        // The immutable snapshot may lag behind an editor buffer. Never use
        // declarations from a stale disk snapshot for a dirty document; the
        // caller can then continue with the legacy/LSP providers.
        let fingerprint_phase = std::time::Instant::now();
        if self
            .file_fingerprints
            .get(&file_id)
            .is_some_and(|fingerprint| *fingerprint != text_fingerprint(text))
        {
            return SemanticDefinitionResult {
                result: DefinitionResult::Unresolved,
                outcome: SemanticDefinitionOutcome::StaleSnapshot,
            };
        }
        fingerprint_us = fingerprint_phase.elapsed().as_micros();
        let scope_phase = std::time::Instant::now();
        let Some(mut scope) = self
            .scopes
            .records
            .iter()
            .filter(|scope| scope.file == Some(file_id) && scope.span.contains(&offset))
            .max_by_key(|scope| scope.span.start)
        else {
            return SemanticDefinitionResult {
                result: DefinitionResult::Unresolved,
                outcome: SemanticDefinitionOutcome::Unresolved,
            };
        };
        if scope.kind == ScopeKind::File {
            // PHP's semicolon namespace form has no explicit body node; use
            // the latest namespace declaration before a top-level cursor.
            if let Some(namespace) = self
                .scopes
                .records
                .iter()
                .filter(|candidate| {
                    candidate.file == Some(file_id)
                        && candidate.kind == ScopeKind::Namespace
                        && candidate.span.start <= offset
                })
                .max_by_key(|candidate| candidate.span.start)
            {
                scope = namespace;
            }
        }
        scope_lookup_us = scope_phase.elapsed().as_micros();
        token_lookup_us = 0;
        let syntax_context_build_us = syntax_context.as_ref().map(|s| s.build_us).unwrap_or(0);
        let syntax_source_resident = syntax_context.is_some();
        let (root, token, is_keyword) = if let Some(syntax) = syntax_context {
            if syntax.token.range.end > text.len()
                || syntax.token.range.start > syntax.token.range.end
                || syntax.token.range.start > offset
                || !text.is_char_boundary(syntax.token.range.start)
            {
                return SemanticDefinitionResult {
                    result: DefinitionResult::Unresolved,
                    outcome: SemanticDefinitionOutcome::IncompleteAst,
                };
            }
            text_clone_us = 0;
            parse_us = 0;
            (syntax.tree, syntax.token, syntax.is_keyword)
        } else {
            let clone_phase = std::time::Instant::now();
            let owned_text = text.to_owned();
            text_clone_us = clone_phase.elapsed().as_micros();
            let parse_phase = std::time::Instant::now();
            let Ok(syntax) = PhpSyntax::parse(owned_text) else {
                return SemanticDefinitionResult {
                    result: DefinitionResult::Unresolved,
                    outcome: SemanticDefinitionOutcome::IncompleteAst,
                };
            };
            parse_us = parse_phase.elapsed().as_micros();
            let token_phase = std::time::Instant::now();
            let Some(token) = syntax.token_at_byte(offset) else {
                return SemanticDefinitionResult {
                    result: DefinitionResult::Unresolved,
                    outcome: SemanticDefinitionOutcome::IncompleteAst,
                };
            };
            token_lookup_us = token_phase.elapsed().as_micros();
            let is_keyword = syntax.is_keyword_at_byte(offset);
            (syntax.tree().clone(), token, is_keyword)
        };
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            let node = root
                .root_node()
                .descendant_for_byte_range(token.range.start, token.range.end);
            eprintln!(
                "[DEFINITION AST] offset={offset} token={:?} kind={} range={:?} node_kind={} node_range={:?} node_text={:?} parent_kind={} parent_range={:?}",
                token.text,
                token.kind,
                token.range,
                node.as_ref().map(|n| n.kind()).unwrap_or("<none>"),
                node.as_ref().map(|n| n.byte_range()),
                node.as_ref()
                    .and_then(|n| n.utf8_text(text.as_bytes()).ok()),
                node.as_ref()
                    .and_then(|n| n.parent())
                    .map(|n| n.kind())
                    .unwrap_or("<none>"),
                node.as_ref()
                    .and_then(|n| n.parent())
                    .map(|n| n.byte_range())
            );
        }
        // Keywords are syntax nodes, not nominal PHP symbols. In particular,
        // never let a statement keyword such as `return` fall through to
        // `resolve_class_name`, where it can become Namespace\\return.
        if is_keyword {
            return SemanticDefinitionResult {
                result: DefinitionResult::Unresolved,
                outcome: SemanticDefinitionOutcome::Unresolved,
            };
        }
        let resolution_phase = std::time::Instant::now();
        let Some(target) = self.definition_for_token(scope.id, root.root_node(), token.range, text)
        else {
            return SemanticDefinitionResult {
                result: DefinitionResult::Unresolved,
                outcome: SemanticDefinitionOutcome::Unresolved,
            };
        };
        resolution_us = resolution_phase.elapsed().as_micros();
        let outcome = match &target {
            DefinitionResult::Resolved(candidate) => match candidate.confidence {
                DefinitionConfidence::Partial => SemanticDefinitionOutcome::Inaccessible,
                _ => SemanticDefinitionOutcome::Resolved,
            },
            DefinitionResult::Candidates(_) => SemanticDefinitionOutcome::Ambiguous,
            DefinitionResult::Deferred(_) => SemanticDefinitionOutcome::DeferredVendor,
            DefinitionResult::Unresolved => SemanticDefinitionOutcome::MissingSymbol,
        };
        let result = SemanticDefinitionResult {
            result: target,
            outcome,
        };
        if profile {
            eprintln!(
                "[DEFINITION PROFILE] file={:?} bytes={} scopes_total={} syntax_source={} syntax_context_build_us={} revision_check_us={} fingerprint_us={} scope_lookup_us={} text_clone_us={} parse_us={} token_lookup_us={} receiver_name_member_us={} total_semantic_us={}",
                file.as_ref(),
                text.len(),
                self.scopes.records.len(),
                if syntax_source_resident {
                    "resident"
                } else {
                    "fallback_parse"
                },
                syntax_context_build_us,
                revision_check_us,
                fingerprint_us,
                scope_lookup_us,
                text_clone_us,
                parse_us,
                token_lookup_us,
                resolution_us,
                profile_started.elapsed().as_micros()
            );
        }
        result
    }

    fn definition_for_token(
        &self,
        scope: ScopeId,
        root: tree_sitter::Node<'_>,
        range: std::ops::Range<usize>,
        text: &str,
    ) -> Option<DefinitionResult> {
        let node = root.descendant_for_byte_range(range.start, range.end)?;
        if node
            .named_child(0)
            .is_some_and(|child| range.start < child.start_byte())
        {
            return None;
        }
        let mut current = Some(node);
        while let Some(candidate) = current {
            if candidate.kind() == "function_call_expression"
                && let Some(function) = candidate.child_by_field_name("function")
                && function.byte_range().contains(&range.start)
            {
                let written = node_text(function, text).trim();
                let resolved = self.resolve_function_name(scope, written)?;
                let ids: Vec<_> = self
                    .symbols_for_fqn(&resolved)
                    .iter()
                    .copied()
                    .filter(|id| {
                        self.symbol(*id)
                            .is_some_and(|symbol| symbol.kind == ProjectSymbolKind::Function)
                    })
                    .collect();
                return Some(self.symbol_ids_to_definition(&ids, DefinitionConfidence::Exact));
            }
            if matches!(
                candidate.kind(),
                "member_call_expression" | "nullsafe_member_call_expression"
            ) {
                let name = candidate
                    .child_by_field_name("name")
                    .or_else(|| candidate.child_by_field_name("constant"))
                    .or_else(|| {
                        let mut cursor = candidate.walk();
                        candidate.named_children(&mut cursor).nth(1)
                    })?;
                if name.byte_range().contains(&range.start) {
                    let object = candidate.child_by_field_name("object")?;
                    let expression = expression_from_ast(object, text)?;
                    let Some(receiver) =
                        ExpressionResolver::new(self, scope).infer_expression_type(&expression)
                    else {
                        // An intermediate method may live in a lazy Vendor
                        // source. Preserve that state so the app can route
                        // the complete chain to the Vendor resolver instead
                        // of turning it into an unconditional Unresolved.
                        if expression_contains_method_call(&expression) {
                            return Some(DefinitionResult::Deferred(
                                "receiver method is deferred to Vendor".into(),
                            ));
                        }
                        return None;
                    };
                    return Some(self.member_result_to_definition(
                        self.member_resolver().resolve_method(
                            scope,
                            &receiver,
                            node_text(name, text),
                            MemberAccess::Instance,
                        ),
                    ));
                }
            }
            if matches!(
                candidate.kind(),
                "member_access_expression" | "nullsafe_member_access_expression"
            ) {
                let name = candidate
                    .child_by_field_name("name")
                    .or_else(|| candidate.child_by_field_name("constant"))
                    .or_else(|| {
                        let mut cursor = candidate.walk();
                        candidate.named_children(&mut cursor).nth(1)
                    })?;
                if name.byte_range().contains(&range.start) {
                    let object = candidate.child_by_field_name("object")?;
                    let expression = expression_from_ast(object, text)?;
                    let receiver =
                        ExpressionResolver::new(self, scope).infer_expression_type(&expression)?;
                    return Some(self.member_result_to_definition(
                        self.member_resolver().resolve_property(
                            scope,
                            &receiver,
                            node_text(name, text),
                        ),
                    ));
                }
            }
            if matches!(
                candidate.kind(),
                "static_call_expression"
                    | "scoped_call_expression"
                    | "class_constant_access_expression"
            ) {
                let name = candidate
                    .child_by_field_name("name")
                    .or_else(|| candidate.child_by_field_name("constant"))
                    .or_else(|| {
                        let mut cursor = candidate.walk();
                        candidate.named_children(&mut cursor).nth(1)
                    })?;
                if name.byte_range().contains(&range.start) {
                    let object = candidate
                        .child_by_field_name("class")
                        .or_else(|| candidate.child_by_field_name("scope"))
                        .or_else(|| candidate.child_by_field_name("object"))
                        .or_else(|| {
                            let mut cursor = candidate.walk();
                            candidate.named_children(&mut cursor).next()
                        })?;
                    let receiver_name = node_text(object, text).trim();
                    let class_name = self.resolve_class_name(scope, receiver_name)?;
                    let resolved_name = class_name.clone();
                    let receiver = DeclaredType::Named {
                        written: receiver_name.to_owned(),
                        resolved: class_name,
                    };
                    let kind = if candidate.kind() == "class_constant_access_expression" {
                        MemberKind::ClassConstant
                    } else {
                        MemberKind::Method
                    };
                    if kind == MemberKind::ClassConstant && node_text(name, text) == "class" {
                        let ids = self.symbols_for_fqn(&resolved_name);
                        return Some(
                            self.symbol_ids_to_definition(ids, DefinitionConfidence::Exact),
                        );
                    }
                    if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                        eprintln!(
                            "[STATIC MEMBER TRACE] ast_kind={} receiver_written={} receiver_fqn={} member_name={} member_kind={:?} lookup_mode={}",
                            candidate.kind(),
                            receiver_name,
                            resolved_name,
                            node_text(name, text),
                            kind,
                            if kind == MemberKind::Method {
                                "static"
                            } else {
                                "constant"
                            }
                        );
                    }
                    let access = if matches!(receiver_name, "self" | "static" | "parent") {
                        MemberAccess::ClassScope
                    } else {
                        MemberAccess::Static
                    };
                    let resolution =
                        match kind {
                            MemberKind::ClassConstant => self
                                .member_resolver()
                                .resolve_class_constant(scope, &receiver, node_text(name, text)),
                            _ => self.member_resolver().resolve_method(
                                scope,
                                &receiver,
                                node_text(name, text),
                                access,
                            ),
                        };
                    return Some(self.member_result_to_definition(resolution));
                }
            }
            current = candidate.parent();
        }
        self.definition_for_named_token(scope, node_text(node, text))
    }

    fn definition_for_named_token(
        &self,
        scope: ScopeId,
        written: &str,
    ) -> Option<DefinitionResult> {
        if let Some(resolved) = self.resolve_constant_name(scope, written) {
            let ids: Vec<_> = self
                .symbols_for_fqn(&resolved)
                .iter()
                .copied()
                .filter(|id| {
                    self.symbol(*id).is_some_and(|symbol| {
                        matches!(
                            symbol.kind,
                            ProjectSymbolKind::Constant | ProjectSymbolKind::ClassConstant
                        )
                    })
                })
                .collect();
            if !ids.is_empty() {
                return Some(self.symbol_ids_to_definition(&ids, DefinitionConfidence::Exact));
            }
        }
        let resolved = self.resolve_class_name(scope, written)?;
        let ids = self.symbols_for_fqn(&resolved);
        if ids.is_empty() {
            return None;
        }
        Some(self.symbol_ids_to_definition(ids, DefinitionConfidence::High))
    }

    fn member_result_to_definition(&self, resolution: MemberResolution) -> DefinitionResult {
        match resolution {
            MemberResolution::Resolved(id) => self
                .symbol_ids_to_definition(std::slice::from_ref(&id), DefinitionConfidence::Exact),
            MemberResolution::Candidates(ids) => {
                self.symbol_ids_to_definition(&ids, DefinitionConfidence::Ambiguous)
            }
            MemberResolution::Deferred(reason) => DefinitionResult::Deferred(reason),
            MemberResolution::ResolvedButInaccessible(id) | MemberResolution::Incompatible(id) => {
                self.symbol_ids_to_definition(
                    std::slice::from_ref(&id),
                    DefinitionConfidence::Partial,
                )
            }
            MemberResolution::Unresolved => DefinitionResult::Unresolved,
        }
    }

    fn symbol_ids_to_definition(
        &self,
        ids: &[SymbolId],
        confidence: DefinitionConfidence,
    ) -> DefinitionResult {
        let mut candidates = Vec::new();
        for id in ids {
            let Some(symbol) = self.symbol(*id) else {
                continue;
            };
            let Some(file) = self.file(symbol.file) else {
                continue;
            };
            candidates.push(DefinitionCandidate {
                location: DefinitionLocation {
                    file: file.path.clone(),
                    span: symbol.range.clone(),
                    origin: file.key.origin.clone(),
                },
                confidence,
            });
        }
        match candidates.len() {
            0 => DefinitionResult::Unresolved,
            1 => DefinitionResult::Resolved(candidates.remove(0)),
            _ => DefinitionResult::Candidates(candidates),
        }
    }

    pub fn scope(&self, id: ScopeId) -> Option<&Scope> {
        self.scopes.records.get(id.0 as usize)
    }

    /// Returns structured visibility information for a partial definition.
    /// Presentation layers can use this without reimplementing PHP visibility
    /// heuristics.
    pub fn explain_inaccessibility(
        &self,
        path: &Path,
        offset: usize,
        result: &DefinitionResult,
    ) -> Option<InaccessibilityInfo> {
        let DefinitionResult::Resolved(candidate) = result else {
            return None;
        };
        if candidate.confidence != DefinitionConfidence::Partial {
            return None;
        }
        let symbol = self.symbols.records.iter().find(|symbol| {
            self.file(symbol.file).is_some_and(|file| file.path == path)
                && symbol.range == candidate.location.span
        })?;
        let owner = symbol.owner.and_then(|id| self.symbol(id))?;
        let declaring_type = owner.fully_qualified_name.clone();
        let access_context = self
            .files
            .records
            .iter()
            .find(|file| file.path == path)
            .and_then(|file| self.scope_at(file.id, offset))
            .and_then(|scope| scope.class_name.clone());
        let reason = match symbol.visibility {
            Visibility::Private => InaccessibilityReason::PrivateMember,
            Visibility::Protected => InaccessibilityReason::ProtectedMember,
            _ => InaccessibilityReason::IncompatibleAccess,
        };
        Some(InaccessibilityInfo {
            reason,
            declaring_type,
            member_name: symbol.name.clone(),
            access_context,
        })
    }

    pub fn lookup_binding(&self, mut scope: ScopeId, name: &str) -> Option<&VariableBinding> {
        loop {
            let current = self.scope(scope)?;
            if let Some(binding) = current
                .bindings
                .iter()
                .rev()
                .find(|binding| binding.name == name)
            {
                return Some(binding);
            }
            scope = current.parent?;
        }
    }

    pub fn resolve_class_name(&self, scope: ScopeId, written: &str) -> Option<String> {
        self.resolve_name(scope, written, ImportKind::Class)
    }

    pub fn resolve_function_name(&self, scope: ScopeId, written: &str) -> Option<String> {
        self.resolve_name(scope, written, ImportKind::Function)
    }

    pub fn resolve_constant_name(&self, scope: ScopeId, written: &str) -> Option<String> {
        self.resolve_name(scope, written, ImportKind::Constant)
    }

    fn resolve_name(&self, mut scope: ScopeId, written: &str, kind: ImportKind) -> Option<String> {
        let written = written.trim();
        if written.is_empty() {
            return None;
        }
        let absolute = written.trim_start_matches('\\');
        if absolute != written {
            return Some(absolute.to_owned());
        }
        if matches!(written, "self" | "static" | "parent") && kind == ImportKind::Class {
            while let Some(current) = self.scope(scope) {
                if let Some(name) = match written {
                    "parent" => current.parent_class.clone(),
                    _ => current.class_name.clone(),
                } {
                    return Some(name);
                }
                scope = current.parent?;
            }
            return Some(written.to_owned());
        }
        let mut namespace = String::new();
        loop {
            let current = self.scope(scope)?;
            if let Some(binding) = current
                .imports
                .get(kind, written.split('\\').next().unwrap_or(written))
            {
                let suffix = written
                    .split_once('\\')
                    .map(|(_, tail)| format!("\\{tail}"))
                    .unwrap_or_default();
                return Some(format!("{}{}", binding.target, suffix));
            }
            if !current.namespace.is_empty() {
                namespace = current.namespace.clone();
            }
            scope = match current.parent {
                Some(parent) => parent,
                None => break,
            };
        }
        if absolute.contains('\\') {
            Some(absolute.to_owned())
        } else if namespace.is_empty() {
            Some(absolute.to_owned())
        } else {
            Some(format!("{namespace}\\{absolute}"))
        }
    }

    pub fn find_usages(&self, symbol: SymbolId, options: FindUsagesOptions) -> FindUsagesResult {
        if self.symbol(symbol).is_none() {
            return FindUsagesResult {
                usages: Vec::new(),
                status: FindUsagesStatus::Stale,
            };
        }
        let mut usages = Vec::new();
        for reference_id in self.references_for_target(symbol) {
            let Some(reference) = self.reference(*reference_id) else {
                continue;
            };
            if !options.include_imports && reference.role == ReferenceRole::Import {
                continue;
            }
            if !options.include_type_references
                && matches!(
                    reference.role,
                    ReferenceRole::Type | ReferenceRole::ReturnType | ReferenceRole::ParameterType
                )
            {
                continue;
            }
            if let Some(location) = self.usage_location(reference, ReferenceConfidence::Exact) {
                usages.push(location);
            }
        }
        let status = if self.references.records.iter().any(|reference| {
            matches!(reference.target, ReferenceTarget::Deferred)
        }) {
            FindUsagesStatus::Deferred
        } else if self.references.ambiguous_references.iter().any(|id| {
            self.reference(*id).is_some_and(|reference| {
                matches!(&reference.target, ReferenceTarget::Candidates(candidates) if candidates.contains(&symbol))
            })
        }) {
            FindUsagesStatus::Ambiguous
        } else {
            FindUsagesStatus::Complete
        };
        FindUsagesResult { usages, status }
    }

    pub fn find_usages_by_key(
        &self,
        key: &PersistentSymbolKey,
        options: FindUsagesOptions,
    ) -> FindUsagesResult {
        let Some(symbol) = self.symbol_id(key) else {
            return FindUsagesResult {
                usages: Vec::new(),
                status: FindUsagesStatus::Stale,
            };
        };
        self.find_usages(symbol, options)
    }

    pub fn find_usages_at(
        &self,
        file: impl AsRef<Path>,
        offset: usize,
        options: FindUsagesOptions,
    ) -> FindUsagesResult {
        let key = PersistentFileKey::workspace(file.as_ref());
        let Some(file_id) = self.file_id(&key) else {
            return FindUsagesResult {
                usages: Vec::new(),
                status: FindUsagesStatus::Stale,
            };
        };
        let candidates: Vec<_> = self
            .symbols_for_file(file_id)
            .iter()
            .copied()
            .filter(|id| {
                self.symbol(*id)
                    .is_some_and(|symbol| symbol.range.contains(&offset))
            })
            .collect();
        if candidates.len() == 1 {
            return self.find_usages(candidates[0], options);
        }
        // A UI hit usually lands on a usage rather than the declaration. Keep
        // this lookup entirely in the public value API: callers never need to
        // obtain or retain SymbolId/ReferenceId values.
        let references: Vec<_> = self
            .references_for_file(file_id)
            .iter()
            .filter_map(|id| self.reference(*id))
            .filter(|reference| reference.span.contains(&offset))
            .collect();
        if references.len() == 1 {
            return match &references[0].target {
                ReferenceTarget::Resolved(symbol) => self.find_usages(*symbol, options),
                ReferenceTarget::Candidates(_) => FindUsagesResult {
                    usages: Vec::new(),
                    status: FindUsagesStatus::Ambiguous,
                },
                ReferenceTarget::Deferred => FindUsagesResult {
                    usages: Vec::new(),
                    status: FindUsagesStatus::Deferred,
                },
                ReferenceTarget::Unresolved | ReferenceTarget::Dynamic => FindUsagesResult {
                    usages: Vec::new(),
                    status: FindUsagesStatus::Stale,
                },
            };
        }
        FindUsagesResult {
            usages: Vec::new(),
            status: FindUsagesStatus::Ambiguous,
        }
    }

    fn usage_location(
        &self,
        reference: &SemanticReference,
        confidence: ReferenceConfidence,
    ) -> Option<UsageLocation> {
        let file = self.file(reference.file)?;
        let source_symbol = reference
            .source_symbol
            .and_then(|id| self.symbol(id))
            .map(|symbol| symbol.key.clone());
        Some(UsageLocation {
            file: file.key.clone(),
            span: reference.span.clone(),
            role: reference.role,
            confidence,
            source_symbol,
            provider: ReferenceProvider::Semantic,
        })
    }

    pub fn save_json(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let persisted = PersistedSnapshot {
            revision: self.revision,
            files: self.files.records.clone(),
            symbols: self.symbols.records.clone(),
            references: self.references.clone(),
            scopes: self.scopes.clone(),
            file_fingerprints: self.file_fingerprints.clone(),
        };
        let bytes = serde_json::to_vec(&persisted)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)
    }

    pub fn load_json(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let persisted: PersistedSnapshot = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        Ok(Self::from_persisted(persisted))
    }

    fn from_persisted(persisted: PersistedSnapshot) -> Self {
        let mut files = FileStore {
            records: persisted.files,
            by_key: HashMap::new(),
        };
        for record in &files.records {
            files.by_key.insert(record.key.clone(), record.id);
        }
        let mut symbols = SymbolStore {
            records: persisted.symbols,
            by_key: HashMap::new(),
            by_fqn: HashMap::new(),
        };
        let mut declarations = DeclarationIndexes::default();
        for symbol in &symbols.records {
            symbols.by_key.insert(symbol.key.clone(), symbol.id);
            symbols
                .by_fqn
                .entry(symbol.fully_qualified_name.clone())
                .or_default()
                .push(symbol.id);
            declarations
                .symbols_by_file
                .entry(symbol.file)
                .or_default()
                .push(symbol.id);
            if let Some(owner) = symbol.owner {
                declarations
                    .members_by_owner
                    .entry(owner)
                    .or_default()
                    .push(symbol.id);
                declarations
                    .members_by_owner_name
                    .entry((
                        owner,
                        symbol.name.trim_start_matches('$').to_owned(),
                        symbol.kind,
                    ))
                    .or_default()
                    .push(symbol.id);
            }
        }
        let mut snapshot = Self {
            revision: persisted.revision,
            files,
            symbols,
            declarations,
            references: persisted.references,
            scopes: persisted.scopes,
            file_fingerprints: persisted.file_fingerprints,
            interface_relations: InterfaceRelationIndexes::default(),
        };
        snapshot.rebuild_interface_relations();
        let active_symbols: HashSet<SymbolId> = snapshot
            .files
            .records
            .iter()
            .flat_map(|file| file.symbols.iter().copied())
            .collect();
        for reference in &snapshot.references.records {
            if let ReferenceTarget::Resolved(symbol) = reference.target {
                debug_assert!(
                    active_symbols.contains(&symbol),
                    "resolved reference points to inactive symbol {symbol:?}"
                );
            }
        }
        snapshot
    }
}

fn expression_contains_method_call(expression: &Expression) -> bool {
    match expression {
        Expression::MethodCall { .. } => true,
        Expression::Parenthesized(inner) => expression_contains_method_call(inner),
        _ => false,
    }
}

fn collect_interface_ancestors(
    snapshot: &SemanticSnapshot,
    extends: &HashMap<SymbolId, Vec<SymbolId>>,
    interface: SymbolId,
    output: &mut HashSet<SymbolId>,
) {
    let mut pending = vec![interface];
    while let Some(current) = pending.pop() {
        if !output.insert(current) {
            continue;
        }
        if let Some(parents) = extends.get(&current) {
            pending.extend(parents.iter().copied().filter(|id| {
                snapshot
                    .symbol(*id)
                    .is_some_and(|symbol| symbol.kind == ProjectSymbolKind::Interface)
            }));
        }
    }
}

/// Mutable construction state. Once `build` returns, the resulting snapshot
/// is immutable and can safely be shared by multiple readers.
#[derive(Debug, Default)]
pub struct SnapshotBuilder {
    revision: SemanticRevision,
    files: FileStore,
    symbols: SymbolStore,
    declarations: DeclarationIndexes,
    references: ReferenceStore,
    scopes: ScopeStore,
    file_fingerprints: HashMap<FileId, u64>,
    pending_files: HashMap<PersistentFileKey, Option<(PathBuf, String)>>,
}

impl SnapshotBuilder {
    pub fn empty(revision: SemanticRevision) -> Self {
        Self {
            revision,
            ..Self::default()
        }
    }

    pub fn from_project_index(index: &ProjectSymbolIndex, revision: SemanticRevision) -> Self {
        let mut builder = Self::empty(revision);
        let mut paths = BTreeSet::new();
        for path in index.indexed_files() {
            paths.insert(path.to_path_buf());
        }
        for path in paths {
            let key = PersistentFileKey::workspace(&path);
            let id = FileId(builder.files.records.len() as u32);
            builder.files.by_key.insert(key.clone(), id);
            builder.files.records.push(FileRecord {
                id,
                key,
                path,
                symbols: Vec::new(),
            });
        }
        let fingerprints = builder
            .files
            .records
            .iter()
            .filter_map(|record| {
                fs::read_to_string(&record.path)
                    .ok()
                    .map(|text| (record.id, text_fingerprint(&text)))
            })
            .collect::<Vec<_>>();
        for (id, fingerprint) in fingerprints {
            builder.file_fingerprints.insert(id, fingerprint);
        }

        let mut source_symbols = index.symbols().to_vec();
        source_symbols.sort_by(|left, right| {
            left.file
                .cmp(&right.file)
                .then(left.range.start.cmp(&right.range.start))
                .then(left.kind.cmp(&right.kind))
                .then(left.fully_qualified_name.cmp(&right.fully_qualified_name))
                .then(left.name.cmp(&right.name))
        });
        let mut occurrence_counts: HashMap<(PersistentFileKey, ProjectSymbolKind, String), u32> =
            HashMap::new();
        for source in &source_symbols {
            let base = (
                PersistentFileKey::workspace(&source.file),
                source.kind,
                source.fully_qualified_name.clone(),
            );
            *occurrence_counts.entry(base).or_default() += 1;
        }
        let mut ordinals: HashMap<(PersistentFileKey, ProjectSymbolKind, String), u32> =
            HashMap::new();

        for source in source_symbols {
            let file_key = PersistentFileKey::workspace(&source.file);
            let file = *builder
                .files
                .by_key
                .get(&file_key)
                .expect("file was collected before symbols");
            let base = (
                file_key.clone(),
                source.kind,
                source.fully_qualified_name.clone(),
            );
            let ordinal = ordinals.entry(base.clone()).or_insert(0);
            let key = PersistentSymbolKey {
                file: file_key.clone(),
                kind: source.kind,
                qualified_name: source.fully_qualified_name.clone(),
                discriminator: (occurrence_counts.get(&base).copied().unwrap_or(1) > 1)
                    .then_some(*ordinal),
            };
            *ordinal += 1;
            let id = SymbolId(builder.symbols.records.len() as u32);
            builder.symbols.by_key.insert(key.clone(), id);
            builder
                .symbols
                .by_fqn
                .entry(source.fully_qualified_name.clone())
                .or_default()
                .push(id);
            builder
                .declarations
                .symbols_by_file
                .entry(file)
                .or_default()
                .push(id);
            builder.files.records[file.0 as usize].symbols.push(id);

            builder.symbols.records.push(SemanticSymbol {
                id,
                key: key.clone(),
                name: source.name,
                fully_qualified_name: source.fully_qualified_name,
                kind: source.kind,
                file,
                range: source.range,
                namespace: source.namespace,
                visibility: source.visibility,
                modifiers: source.modifiers,
                parameters: source.parameters,
                return_type: source.return_type,
                owner: None,
                owner_key: None,
            });
        }

        let mut owners_by_file_and_name: HashMap<(FileId, String), Vec<SymbolId>> = HashMap::new();
        let owner_keys: HashMap<SymbolId, PersistentSymbolKey> = builder
            .symbols
            .records
            .iter()
            .map(|symbol| (symbol.id, symbol.key.clone()))
            .collect();
        for symbol in &builder.symbols.records {
            if matches!(
                symbol.kind,
                ProjectSymbolKind::Class
                    | ProjectSymbolKind::Interface
                    | ProjectSymbolKind::Trait
                    | ProjectSymbolKind::Enum
            ) {
                owners_by_file_and_name
                    .entry((symbol.file, symbol.fully_qualified_name.clone()))
                    .or_default()
                    .push(symbol.id);
            }
        }
        for symbol in &mut builder.symbols.records {
            let Some((owner_name, _)) = symbol.fully_qualified_name.rsplit_once("::") else {
                continue;
            };
            let Some(candidates) = owners_by_file_and_name
                .get(&(symbol.file, owner_name.to_owned()))
                .filter(|candidates| candidates.len() == 1)
            else {
                continue;
            };
            let owner = candidates[0];
            symbol.owner = Some(owner);
            symbol.owner_key = owner_keys.get(&owner).cloned();
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!(
                    "[MEMBER INDEX TRACE] method_name={} method_fqn={} method_symbol_id={:?} method_file={} derived_owner_fqn={} derived_owner_symbol_id={:?} symbol_kind={:?} inserted_member_key=({:?},{},{:?})",
                    symbol.name,
                    symbol.fully_qualified_name,
                    symbol.id,
                    symbol.file.0,
                    owner_name,
                    owner,
                    symbol.kind,
                    owner,
                    symbol.name.trim_start_matches('$'),
                    symbol.kind
                );
            }
            builder
                .declarations
                .members_by_owner
                .entry(owner)
                .or_default()
                .push(symbol.id);
            builder
                .declarations
                .members_by_owner_name
                .entry((
                    owner,
                    symbol.name.trim_start_matches('$').to_owned(),
                    symbol.kind,
                ))
                .or_default()
                .push(symbol.id);
        }
        builder.populate_scopes_from_files();
        builder
    }

    /// Starts a new revision from an immutable snapshot. File replacements
    /// are applied in a batch by `finish`; the base snapshot is never mutated.
    pub fn from_snapshot(base: &SemanticSnapshot) -> Self {
        Self {
            revision: SemanticRevision(base.revision.0.saturating_add(1)),
            files: base.files.clone(),
            symbols: base.symbols.clone(),
            declarations: base.declarations.clone(),
            references: base.references.clone(),
            scopes: base.scopes.clone(),
            file_fingerprints: base.file_fingerprints.clone(),
            pending_files: HashMap::new(),
        }
    }

    /// Queues an explicitly Workspace-owned file replacement. Multiple calls
    /// are committed as one snapshot, avoiding intermediate revisions.
    pub fn replace_workspace_file(&mut self, path: impl AsRef<Path>, text: impl Into<String>) {
        let path = path.as_ref().to_path_buf();
        let text = text.into();
        self.pending_files
            .insert(PersistentFileKey::workspace(&path), Some((path, text)));
    }

    /// Queues removal of a workspace file and all references originating in it.
    pub fn remove_file(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref().to_path_buf();
        self.pending_files
            .insert(PersistentFileKey::workspace(&path), None);
    }

    /// Builds the lexical portion of a snapshot directly from a PHP source
    /// buffer. Tree-sitter supplies the structure; malformed nodes are simply
    /// skipped so editor buffers can be analyzed incrementally.
    pub fn from_php_text(path: impl AsRef<Path>, text: &str, revision: SemanticRevision) -> Self {
        let mut builder = Self::empty(revision);
        let path = path.as_ref().to_path_buf();
        let key = PersistentFileKey::workspace(&path);
        let file = FileId(0);
        builder.files.by_key.insert(key.clone(), file);
        builder.files.records.push(FileRecord {
            id: file,
            key,
            path,
            symbols: Vec::new(),
        });
        builder
            .file_fingerprints
            .insert(file, text_fingerprint(text));
        let file_scope = builder.new_scope(None, ScopeKind::File, String::new(), None, None, false);
        builder.set_scope_file_span(file_scope, file, 0..text.len());
        if let Ok(syntax) = PhpSyntax::parse(text.to_owned()) {
            extract_scopes(&mut builder, syntax.tree().root_node(), file_scope, text);
            extract_assignments(&mut builder, syntax.tree().root_node(), text, file);
        }
        builder
    }

    fn new_scope(
        &mut self,
        parent: Option<ScopeId>,
        kind: ScopeKind,
        namespace: String,
        class_name: Option<String>,
        parent_class: Option<String>,
        is_static_method: bool,
    ) -> ScopeId {
        let id = ScopeId(self.scopes.records.len() as u32);
        self.scopes.records.push(Scope {
            id,
            parent,
            kind,
            owner: None,
            namespace,
            class_name,
            parent_class,
            interfaces_extended: Vec::new(),
            traits_used: Vec::new(),
            is_static_method,
            imports: ImportTable::default(),
            bindings: Vec::new(),
            file: None,
            span: 0..0,
        });
        id
    }

    fn set_scope_file_span(&mut self, id: ScopeId, file: FileId, span: std::ops::Range<usize>) {
        if let Some(scope) = self.scopes.records.get_mut(id.0 as usize) {
            scope.file = Some(file);
            scope.span = span;
        }
    }

    fn populate_scopes_from_files(&mut self) {
        let files: Vec<PathBuf> = self
            .files
            .records
            .iter()
            .map(|record| record.path.clone())
            .collect();
        for path in files {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let file_scope =
                self.new_scope(None, ScopeKind::File, String::new(), None, None, false);
            let file_id = self
                .files
                .records
                .iter()
                .find(|record| record.path == path)
                .map(|file| file.id);
            if let Some(file_id) = file_id {
                self.set_scope_file_span(file_scope, file_id, 0..text.len());
            }
            if let Ok(syntax) = PhpSyntax::parse(text.clone()) {
                extract_scopes(self, syntax.tree().root_node(), file_scope, &text);
                if let Some(file_id) = file_id {
                    extract_assignments(self, syntax.tree().root_node(), &text, file_id);
                }
            }
        }
        let semantic = self.snapshot_view();
        let files: Vec<(FileId, PathBuf)> = semantic
            .files
            .records
            .iter()
            .map(|record| (record.id, record.path.clone()))
            .collect();
        for (file, path) in files {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(syntax) = PhpSyntax::parse(text.clone()) else {
                continue;
            };
            extract_references(&semantic, self, file, syntax.tree().root_node(), &text);
        }
    }

    fn snapshot_view(&self) -> SemanticSnapshot {
        SemanticSnapshot {
            revision: self.revision,
            files: self.files.clone(),
            symbols: self.symbols.clone(),
            declarations: self.declarations.clone(),
            references: ReferenceStore::default(),
            scopes: self.scopes.clone(),
            file_fingerprints: self.file_fingerprints.clone(),
            interface_relations: InterfaceRelationIndexes::default(),
        }
    }

    pub fn build(mut self) -> SemanticSnapshot {
        self.apply_pending_files();
        let total_scopes = self.scopes.records.len();
        let active_scopes = self
            .scopes
            .records
            .iter()
            .filter(|scope| scope.file.is_some())
            .count();
        if should_compact_scopes(total_scopes, active_scopes) {
            let started = std::time::Instant::now();
            self.compact_scopes();
            if std::env::var_os("AXIOM_DEBUG_SEMANTIC").is_some() {
                eprintln!(
                    "[SCOPE COMPACTION] revision={} total_before={} active_before={} total_after={} ratio={} elapsed_us={}",
                    self.revision.0,
                    total_scopes,
                    active_scopes,
                    self.scopes.records.len(),
                    SCOPE_COMPACTION_RATIO,
                    started.elapsed().as_micros()
                );
            }
        }
        let mut snapshot = SemanticSnapshot {
            revision: self.revision,
            files: self.files,
            symbols: self.symbols,
            declarations: self.declarations,
            references: self.references,
            scopes: self.scopes,
            file_fingerprints: self.file_fingerprints,
            interface_relations: InterfaceRelationIndexes::default(),
        };
        snapshot.rebuild_interface_relations();
        snapshot
    }

    pub fn finish(self) -> SemanticSnapshot {
        self.build()
    }

    /// Compacts detached scope tombstones into a dense arena. Scope IDs are
    /// local to a snapshot and may therefore change; symbol and file IDs are
    /// deliberately left untouched. This operation is explicit for now and
    /// is not invoked automatically by `finish`.
    pub fn compact_scopes(&mut self) {
        let active = self
            .scopes
            .records
            .iter()
            .map(|scope| scope.file.is_some())
            .collect::<Vec<_>>();
        if active.iter().all(|is_active| *is_active) {
            return;
        }

        let mut old_to_new = vec![None; self.scopes.records.len()];
        let mut compacted = Vec::with_capacity(active.iter().filter(|active| **active).count());
        for (old_index, scope) in self.scopes.records.iter().enumerate() {
            if !active[old_index] {
                continue;
            }
            let new_id = ScopeId(compacted.len() as u32);
            old_to_new[old_index] = Some(new_id);
            let mut copy = scope.clone();
            copy.id = new_id;
            compacted.push(copy);
        }

        for scope in &mut compacted {
            if let Some(parent) = scope.parent {
                let mapped = old_to_new.get(parent.0 as usize).copied().flatten();
                assert!(
                    mapped.is_some(),
                    "active scope {:?} points to inactive/invalid parent {:?}",
                    scope.id,
                    parent
                );
                scope.parent = mapped;
            }
        }

        for reference in &mut self.references.records {
            let mapped = old_to_new
                .get(reference.source_scope.0 as usize)
                .copied()
                .flatten();
            assert!(
                mapped.is_some(),
                "reference {:?} points to inactive/invalid scope {:?}",
                reference.id,
                reference.source_scope
            );
            reference.source_scope = mapped.expect("checked above");
        }

        self.scopes.records = compacted;
    }

    fn apply_pending_files(&mut self) {
        if self.pending_files.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.pending_files);
        let changed_ids: std::collections::HashSet<_> = changes
            .keys()
            .filter_map(|key| self.files.by_key.get(key).copied())
            .collect();

        // Collect every symbol whose identity will no longer be active. This
        // includes replacements (renames/removals), not only remove_file.
        // References in unrelated files must not retain a Resolved target
        // pointing at one of these IDs.
        let mut removed_symbols = HashSet::new();
        for (key, change) in &changes {
            let Some(file) = self.files.by_key.get(key).copied() else {
                continue;
            };
            let old_ids = self
                .files
                .records
                .get(file.0 as usize)
                .map(|record| record.symbols.clone())
                .unwrap_or_default();
            match change {
                None => removed_symbols.extend(old_ids),
                Some((path, text)) => {
                    let mut replacement_keys = HashSet::new();
                    let mut source_index = ProjectSymbolIndex::new();
                    let _ = source_index.index_file_text(path, text);
                    for symbol in source_index.symbols() {
                        replacement_keys.insert((symbol.kind, symbol.fully_qualified_name.clone()));
                    }
                    for id in old_ids {
                        let Some(symbol) = self.symbols.records.get(id.0 as usize) else {
                            removed_symbols.insert(id);
                            continue;
                        };
                        if !replacement_keys
                            .contains(&(symbol.kind, symbol.fully_qualified_name.clone()))
                        {
                            removed_symbols.insert(id);
                        }
                    }
                }
            }
        }

        // Rebuild the dense reference arena while retaining every unrelated
        // file's records. This removes stale reverse-index entries and keeps
        // ReferenceId deliberately local to the new snapshot.
        let old_records = std::mem::take(&mut self.references.records);
        self.references = ReferenceStore::default();
        for reference in old_records {
            if changed_ids.contains(&reference.file) {
                continue;
            }
            let target = match reference.target {
                ReferenceTarget::Resolved(id) if removed_symbols.contains(&id) => {
                    ReferenceTarget::Unresolved
                }
                ReferenceTarget::Candidates(ids) => {
                    let ids: Vec<_> = ids
                        .into_iter()
                        .filter(|id| !removed_symbols.contains(id))
                        .collect();
                    match ids.len() {
                        0 => ReferenceTarget::Unresolved,
                        1 => ReferenceTarget::Resolved(ids[0]),
                        _ => ReferenceTarget::Candidates(ids),
                    }
                }
                target => target,
            };
            add_reference(
                self,
                reference.file,
                reference.span,
                reference.source_scope,
                reference.source_symbol,
                reference.role,
                target,
            );
        }

        let mut extracted = Vec::new();
        for (key, change) in changes {
            let removed = change.is_none();
            let file = match change {
                Some((path, text)) => {
                    let file = if let Some(file) = self.files.by_key.get(&key).copied() {
                        if let Some(record) = self.files.records.get_mut(file.0 as usize) {
                            record.path = path.clone();
                        }
                        file
                    } else {
                        let file = FileId(self.files.records.len() as u32);
                        self.files.by_key.insert(key.clone(), file);
                        self.files.records.push(FileRecord {
                            id: file,
                            key: key.clone(),
                            path: path.clone(),
                            symbols: Vec::new(),
                        });
                        file
                    };
                    for scope in &mut self.scopes.records {
                        if scope.file == Some(file) {
                            scope.file = None;
                        }
                    }
                    let file_scope =
                        self.new_scope(None, ScopeKind::File, String::new(), None, None, false);
                    self.set_scope_file_span(file_scope, file, 0..text.len());
                    self.file_fingerprints.insert(file, text_fingerprint(&text));
                    self.replace_symbols_for_file(file, &path, &text);
                    if let Ok(syntax) = PhpSyntax::parse(text.clone()) {
                        extract_scopes(self, syntax.tree().root_node(), file_scope, &text);
                        extract_assignments(self, syntax.tree().root_node(), &text, file);
                        extracted.push((file, path, text, syntax));
                    }
                    Some(file)
                }
                None => {
                    let file = self.files.by_key.remove(&key);
                    if let Some(file) = file {
                        let symbol_ids = self
                            .files
                            .records
                            .get(file.0 as usize)
                            .map(|record| record.symbols.clone())
                            .unwrap_or_default();
                        for id in &symbol_ids {
                            if let Some((key, fqn)) =
                                self.symbols.records.get(id.0 as usize).map(|symbol| {
                                    (symbol.key.clone(), symbol.fully_qualified_name.clone())
                                })
                            {
                                self.symbols.by_key.remove(&key);
                                if let Some(ids) = self.symbols.by_fqn.get_mut(&fqn) {
                                    ids.retain(|candidate| candidate != id);
                                    if ids.is_empty() {
                                        self.symbols.by_fqn.remove(&fqn);
                                    }
                                }
                            }
                        }
                        self.declarations.symbols_by_file.remove(&file);
                        if let Some(record) = self.files.records.get_mut(file.0 as usize) {
                            // Keep the arena slot for stable IDs, but make it
                            // semantically inactive and invisible to indexes.
                            record.symbols.clear();
                        }
                    }
                    file
                }
            };
            if let Some(file) = file {
                if removed {
                    self.file_fingerprints.remove(&file);
                    for scope in &mut self.scopes.records {
                        if scope.file == Some(file) {
                            scope.file = None;
                        }
                    }
                }
            }
        }
        let semantic = self.snapshot_view();
        for (file, _path, text, syntax) in extracted {
            extract_references(&semantic, self, file, syntax.tree().root_node(), &text);
        }
        self.rebuild_member_indexes();
    }

    fn replace_symbols_for_file(&mut self, file: FileId, path: &Path, text: &str) {
        let old_ids = self.files.records[file.0 as usize].symbols.clone();
        let mut reusable = HashMap::new();
        for old in &old_ids {
            if let Some(symbol) = self.symbols.records.get(old.0 as usize) {
                reusable.insert((symbol.kind, symbol.fully_qualified_name.clone()), *old);
                self.symbols.by_key.remove(&symbol.key);
                if let Some(ids) = self.symbols.by_fqn.get_mut(&symbol.fully_qualified_name) {
                    ids.retain(|id| id != old);
                }
            }
            if let Some(ids) = self.declarations.symbols_by_file.get_mut(&file) {
                ids.retain(|id| id != old);
            }
        }
        self.declarations.members_by_owner.retain(|owner, members| {
            if old_ids.contains(owner) {
                return false;
            }
            members.retain(|id| !old_ids.contains(id));
            !members.is_empty()
        });
        self.declarations
            .members_by_owner_name
            .retain(|(owner, _, _), members| {
                if old_ids.contains(owner) {
                    return false;
                }
                members.retain(|id| !old_ids.contains(id));
                !members.is_empty()
            });
        self.files.records[file.0 as usize].symbols.clear();

        let mut source_index = ProjectSymbolIndex::new();
        let _ = source_index.index_file_text(path, text);
        for source in source_index.symbols().iter().cloned() {
            let id = reusable
                .remove(&(source.kind, source.fully_qualified_name.clone()))
                .unwrap_or(SymbolId(self.symbols.records.len() as u32));
            let key = self
                .symbols
                .records
                .get(id.0 as usize)
                .map(|symbol| symbol.key.clone())
                .unwrap_or(PersistentSymbolKey {
                    file: PersistentFileKey::workspace(path),
                    kind: source.kind,
                    qualified_name: source.fully_qualified_name.clone(),
                    discriminator: Some(id.0),
                });
            self.symbols.by_key.insert(key.clone(), id);
            self.symbols
                .by_fqn
                .entry(source.fully_qualified_name.clone())
                .or_default()
                .push(id);
            self.declarations
                .symbols_by_file
                .entry(file)
                .or_default()
                .push(id);
            self.files.records[file.0 as usize].symbols.push(id);
            let replacement = SemanticSymbol {
                id,
                key,
                name: source.name,
                fully_qualified_name: source.fully_qualified_name,
                kind: source.kind,
                file,
                range: source.range,
                namespace: source.namespace,
                visibility: source.visibility,
                modifiers: source.modifiers,
                parameters: source.parameters,
                return_type: source.return_type,
                owner: None,
                owner_key: None,
            };
            if let Some(slot) = self.symbols.records.get_mut(id.0 as usize) {
                *slot = replacement;
            } else {
                self.symbols.records.push(replacement);
            }
        }
        self.rebuild_member_indexes();
    }

    fn rebuild_member_indexes(&mut self) {
        self.declarations.members_by_owner.clear();
        self.declarations.members_by_owner_name.clear();
        let active: HashSet<SymbolId> = self
            .files
            .records
            .iter()
            .flat_map(|record| record.symbols.iter().copied())
            .collect();
        for symbol in &mut self.symbols.records {
            if active.contains(&symbol.id) {
                symbol.owner = None;
                symbol.owner_key = None;
            }
        }
        let owners: HashMap<(FileId, String), SymbolId> = self
            .symbols
            .records
            .iter()
            .filter(|s| {
                active.contains(&s.id)
                    && matches!(
                        s.kind,
                        ProjectSymbolKind::Class
                            | ProjectSymbolKind::Interface
                            | ProjectSymbolKind::Trait
                            | ProjectSymbolKind::Enum
                    )
            })
            .map(|s| ((s.file, s.fully_qualified_name.clone()), s.id))
            .collect();
        let owner_keys: HashMap<SymbolId, PersistentSymbolKey> = self
            .symbols
            .records
            .iter()
            .filter(|s| active.contains(&s.id))
            .map(|s| (s.id, s.key.clone()))
            .collect();
        for symbol in &mut self.symbols.records {
            if !active.contains(&symbol.id) {
                continue;
            }
            let Some((owner_name, _)) = symbol.fully_qualified_name.rsplit_once("::") else {
                continue;
            };
            let Some(&owner) = owners.get(&(symbol.file, owner_name.to_owned())) else {
                continue;
            };
            symbol.owner = Some(owner);
            symbol.owner_key = owner_keys.get(&owner).cloned();
            self.declarations
                .members_by_owner
                .entry(owner)
                .or_default()
                .push(symbol.id);
            self.declarations
                .members_by_owner_name
                .entry((
                    owner,
                    symbol.name.trim_start_matches('$').to_owned(),
                    symbol.kind,
                ))
                .or_default()
                .push(symbol.id);
        }
    }
}

/// Minimal publication façade. It intentionally does not own or alter the
/// existing project/vendor/runtime indexes yet.
#[derive(Debug, Clone)]
pub struct SemanticEngine {
    current: Arc<RwLock<Arc<SemanticSnapshot>>>,
}

impl Default for SemanticEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticEngine {
    pub fn new() -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(SemanticSnapshot::default()))),
        }
    }

    pub fn from_snapshot(snapshot: SemanticSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(snapshot))),
        }
    }

    pub fn snapshot(&self) -> Arc<SemanticSnapshot> {
        self.current
            .read()
            .expect("semantic snapshot lock poisoned")
            .clone()
    }

    /// High-level query boundary for editor/workspace consumers. The caller
    /// supplies only a file path and byte offset; compact semantic IDs remain
    /// an implementation detail of the snapshot.
    pub fn find_usages_at(
        &self,
        file: impl AsRef<Path>,
        offset: usize,
        options: FindUsagesOptions,
    ) -> FindUsagesResult {
        self.snapshot().find_usages_at(file, offset, options)
    }

    pub fn publish(&self, snapshot: Arc<SemanticSnapshot>) -> bool {
        let mut current = self
            .current
            .write()
            .expect("semantic snapshot lock poisoned");
        if snapshot.revision <= current.revision {
            return false;
        }
        *current = snapshot;
        true
    }

    /// Future workers can use this API to reject results based on an obsolete
    /// snapshot. The current revision must match the captured base revision.
    pub fn publish_from(
        &self,
        base_revision: SemanticRevision,
        snapshot: Arc<SemanticSnapshot>,
    ) -> bool {
        let mut current = self
            .current
            .write()
            .expect("semantic snapshot lock poisoned");
        if current.revision != base_revision || snapshot.revision <= current.revision {
            return false;
        }
        *current = snapshot;
        true
    }
}

fn node_text<'a>(node: tree_sitter::Node<'a>, text: &'a str) -> &'a str {
    node.utf8_text(text.as_bytes()).unwrap_or("")
}

fn collect_trait_names(
    builder: &SnapshotBuilder,
    node: tree_sitter::Node<'_>,
    scope: ScopeId,
    text: &str,
    output: &mut Vec<String>,
) {
    if node.kind() == "use_declaration" {
        for name in node_text(node, text)
            .trim()
            .trim_end_matches(';')
            .strip_prefix("use")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            output.push(resolve_builder_name(
                builder,
                scope,
                name,
                ImportKind::Class,
            ));
        }
        return;
    }
    if matches!(
        node.kind(),
        "function_definition" | "method_declaration" | "anonymous_function"
    ) {
        return;
    }
    for child in node.named_children(&mut node.walk()) {
        collect_trait_names(builder, child, scope, text, output);
    }
}

fn extract_scopes(
    builder: &mut SnapshotBuilder,
    node: tree_sitter::Node<'_>,
    scope: ScopeId,
    text: &str,
) {
    if node.kind() == "program" {
        // Unbracketed PHP namespaces are sibling lexical regions. Do not use
        // the previously active namespace as the parent of the next one.
        let namespace_parent = scope;
        let mut active = namespace_parent;
        for child in node.named_children(&mut node.walk()) {
            if child.kind() == "namespace_definition" {
                active = create_namespace_scope(builder, child, namespace_parent, text);
            } else {
                extract_scopes(builder, child, active, text);
            }
        }
        return;
    }
    let kind = node.kind();
    match kind {
        "namespace_use_declaration" => {
            if let Some(current) = builder.scopes.records.get_mut(scope.0 as usize) {
                parse_imports(node_text(node, text), &mut current.imports);
            }
            return;
        }
        "namespace_definition" => {
            let child = create_namespace_scope(builder, node, scope, text);
            for child_node in node.named_children(&mut node.walk()) {
                if child_node.kind() != "namespace_name" {
                    extract_scopes(builder, child_node, child, text);
                }
            }
            return;
        }
        "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(n, text).to_owned());
            let namespace = nearest_namespace(builder, scope);
            let name = name.map(|name| {
                if namespace.is_empty() || name.contains('\\') {
                    name
                } else {
                    format!("{namespace}\\{name}")
                }
            });
            let parent_class = node
                .child_by_field_name("base_clause")
                .or_else(|| {
                    node.named_children(&mut node.walk())
                        .find(|child| matches!(child.kind(), "base_clause" | "extends_clause"))
                })
                .map(|n| {
                    resolve_builder_name(
                        builder,
                        scope,
                        node_text(n, text)
                            .trim()
                            .trim_start_matches("extends ")
                            .trim(),
                        ImportKind::Class,
                    )
                });
            let scope_kind = match kind {
                "interface_declaration" => ScopeKind::Interface,
                "trait_declaration" => ScopeKind::Trait,
                "enum_declaration" => ScopeKind::Enum,
                _ => ScopeKind::Class,
            };
            let child = builder.new_scope(
                Some(scope),
                scope_kind,
                namespace,
                name.clone(),
                parent_class,
                false,
            );
            if kind == "interface_declaration" {
                if let Some(base) = node.child_by_field_name("base_clause").or_else(|| {
                    node.named_children(&mut node.walk())
                        .find(|child| matches!(child.kind(), "base_clause" | "extends_clause"))
                }) {
                    let parents = node_text(base, text)
                        .trim()
                        .strip_prefix("extends")
                        .unwrap_or_default()
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(|name| resolve_builder_name(builder, scope, name, ImportKind::Class))
                        .collect();
                    builder.scopes.records[child.0 as usize].interfaces_extended = parents;
                }
            }
            if matches!(kind, "class_declaration" | "trait_declaration") {
                let mut traits = Vec::new();
                collect_trait_names(builder, node, scope, text, &mut traits);
                if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                    eprintln!(
                        "[TRAIT RELATION BUILD] phase=extract owner={:?} traits_resolved={traits:?}",
                        name
                    );
                }
                builder.scopes.records[child.0 as usize].traits_used = traits;
            }
            mark_scope(builder, child, node);
            for child_node in node.named_children(&mut node.walk()) {
                extract_scopes(builder, child_node, child, text);
            }
            return;
        }
        "function_definition" | "method_declaration" | "anonymous_function" | "arrow_function" => {
            let scope_kind = match kind {
                "method_declaration" => ScopeKind::Method,
                "anonymous_function" => ScopeKind::Closure,
                "arrow_function" => ScopeKind::ArrowFunction,
                _ => ScopeKind::Function,
            };
            let class_name = nearest_class(builder, scope).and_then(|s| s.class_name.clone());
            let parent_class = nearest_class(builder, scope).and_then(|s| s.parent_class.clone());
            let header_end = node
                .child_by_field_name("body")
                .map(|body| body.start_byte())
                .unwrap_or(node.end_byte());
            let header = &text[node.start_byte().min(text.len())..header_end.min(text.len())];
            let is_static = scope_kind == ScopeKind::Method
                && header
                    .split('{')
                    .next()
                    .is_some_and(|h| h.split_whitespace().any(|word| word == "static"));
            let child = builder.new_scope(
                Some(scope),
                scope_kind,
                nearest_namespace(builder, scope),
                class_name,
                parent_class,
                is_static,
            );
            mark_scope(builder, child, node);
            add_parameters(builder, child, node.child_by_field_name("parameters"), text);
            if scope_kind == ScopeKind::Method && !is_static {
                let class = builder.scopes.records[child.0 as usize].class_name.clone();
                if let Some(class) = class {
                    builder.scopes.records[child.0 as usize].bindings.insert(
                        0,
                        VariableBinding {
                            name: "$this".to_owned(),
                            declaration_span: node.start_byte()..node.start_byte(),
                            declared_type: Some(DeclaredType::Named {
                                written: "$this".to_owned(),
                                resolved: class,
                            }),
                        },
                    );
                }
            }
            if let Some(body) = node
                .child_by_field_name("body")
                .or_else(|| node.child_by_field_name("body_expression"))
            {
                for child_node in body.named_children(&mut body.walk()) {
                    extract_scopes(builder, child_node, child, text);
                }
            }
            return;
        }
        _ => {}
    }
    for child in node.named_children(&mut node.walk()) {
        extract_scopes(builder, child, scope, text);
    }
}

fn create_namespace_scope(
    builder: &mut SnapshotBuilder,
    node: tree_sitter::Node<'_>,
    parent: ScopeId,
    text: &str,
) -> ScopeId {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, text).trim().trim_matches('\\').to_owned())
        .unwrap_or_default();
    let child = builder.new_scope(Some(parent), ScopeKind::Namespace, name, None, None, false);
    mark_scope(builder, child, node);
    if let Some(body) = node.child_by_field_name("body") {
        for child_node in body.named_children(&mut body.walk()) {
            extract_scopes(builder, child_node, child, text);
        }
    }
    child
}

fn mark_scope(builder: &mut SnapshotBuilder, id: ScopeId, node: tree_sitter::Node<'_>) {
    let file = builder
        .scopes
        .records
        .get(id.0 as usize)
        .and_then(|scope| scope.parent)
        .and_then(|parent| builder.scopes.records.get(parent.0 as usize))
        .and_then(|scope| scope.file);
    if let Some(scope) = builder.scopes.records.get_mut(id.0 as usize) {
        scope.file = file;
        scope.span = node.byte_range();
    }
}

fn extract_assignments(
    builder: &mut SnapshotBuilder,
    root: tree_sitter::Node<'_>,
    text: &str,
    file: FileId,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "assignment_expression" {
            let Some(left) = node.child_by_field_name("left") else {
                stack.extend(node.named_children(&mut node.walk()));
                continue;
            };
            let Some(right) = node.child_by_field_name("right") else {
                stack.extend(node.named_children(&mut node.walk()));
                continue;
            };
            let name = node_text(left, text).trim();
            if name.starts_with('$') {
                if let Some(scope) = assignment_scope(builder, file, node.start_byte()) {
                    let right_text = node_text(right, text).trim();
                    let raw_type = if right.kind() == "object_creation_expression" {
                        right
                            .child_by_field_name("class")
                            .map(|class| node_text(class, text).trim().to_owned())
                            .or_else(|| {
                                right_text.strip_prefix("new ").map(|value| {
                                    value.split('(').next().unwrap_or(value).trim().to_owned()
                                })
                            })
                    } else if right.kind() == "function_call_expression" {
                        right
                            .child_by_field_name("function")
                            .map(|function| node_text(function, text).trim().to_owned())
                            .and_then(|function| {
                                let resolved = resolve_builder_name(
                                    builder,
                                    scope,
                                    &function,
                                    ImportKind::Function,
                                );
                                builder.symbols.by_fqn.get(&resolved).and_then(|ids| {
                                    ids.iter()
                                        .find_map(|id| builder.symbols.records.get(id.0 as usize))
                                        .and_then(|symbol| symbol.return_type.clone())
                                })
                            })
                    } else {
                        None
                    };
                    if let Some(raw_type) = raw_type {
                        let binding = VariableBinding {
                            name: name.to_owned(),
                            declaration_span: left.byte_range(),
                            declared_type: Some(declared_type(&raw_type, builder, scope)),
                        };
                        let bindings = &mut builder.scopes.records[scope.0 as usize].bindings;
                        if let Some(existing) =
                            bindings.iter_mut().find(|existing| existing.name == name)
                        {
                            // AST traversal is stack-based, so assignments
                            // may be visited in reverse source order. Keep
                            // the binding from the latest source assignment
                            // regardless of traversal order.
                            if existing.declaration_span.start <= binding.declaration_span.start {
                                *existing = binding;
                            }
                        } else {
                            bindings.push(binding);
                        }
                    }
                }
            }
        }
        stack.extend(node.named_children(&mut node.walk()));
    }
}

fn extract_references(
    snapshot: &SemanticSnapshot,
    builder: &mut SnapshotBuilder,
    file: FileId,
    root: tree_sitter::Node<'_>,
    text: &str,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let Some(scope) = snapshot.scope_at(file, node.start_byte()) else {
            stack.extend(node.named_children(&mut node.walk()));
            continue;
        };
        let source_scope = scope.id;
        let source_symbol = source_symbol_for_scope(snapshot, file, scope);

        // These nodes are intentionally handled before the generic traversal:
        // their names are references to types, but not ordinary `Type` uses.
        if matches!(node.kind(), "name" | "qualified_name" | "relative_name") {
            if let Some(parent) = node.parent() {
                let parent_text = node_text(parent, text);
                if parent_text.contains(" instanceof ")
                    && node.start_byte()
                        > parent_text
                            .find("instanceof")
                            .map(|offset| parent.start_byte() + offset)
                            .unwrap_or(usize::MAX)
                {
                    add_reference(
                        builder,
                        file,
                        node.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::Instanceof,
                        snapshot_target_for_name(
                            snapshot,
                            source_scope,
                            node_text(node, text),
                            ImportKind::Class,
                        ),
                    );
                }
            }
        }

        match node.kind() {
            "namespace_use_declaration" => {
                add_import_references(
                    snapshot,
                    builder,
                    file,
                    source_scope,
                    source_symbol,
                    node,
                    text,
                );
            }
            "object_creation_expression" => {
                if let Some(class) = object_creation_class(node) {
                    add_reference(
                        builder,
                        file,
                        class.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::Instantiation,
                        snapshot_target_for_name(
                            snapshot,
                            source_scope,
                            node_text(class, text),
                            ImportKind::Class,
                        ),
                    );
                }
            }
            "base_clause" => {
                add_named_type_references(
                    snapshot,
                    builder,
                    file,
                    source_scope,
                    source_symbol,
                    node,
                    ReferenceRole::Extends,
                    text,
                );
            }
            "class_interface_clause" => {
                add_named_type_references(
                    snapshot,
                    builder,
                    file,
                    source_scope,
                    source_symbol,
                    node,
                    ReferenceRole::Implements,
                    text,
                );
            }
            "use_declaration" => {
                add_named_type_references(
                    snapshot,
                    builder,
                    file,
                    source_scope,
                    source_symbol,
                    node,
                    ReferenceRole::TraitUse,
                    text,
                );
            }
            "catch_clause" => {
                if let Some(type_node) = node.child_by_field_name("type") {
                    add_type_references(
                        snapshot,
                        builder,
                        file,
                        source_scope,
                        source_symbol,
                        type_node,
                        ReferenceRole::CatchType,
                        text,
                    );
                }
            }
            "attribute" => {
                if let Some(name) = node.named_children(&mut node.walk()).next() {
                    add_reference(
                        builder,
                        file,
                        name.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::Attribute,
                        snapshot_target_for_name(
                            snapshot,
                            source_scope,
                            node_text(name, text),
                            ImportKind::Class,
                        ),
                    );
                }
            }
            "function_call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    let written = node_text(function, text).trim();
                    let target = if written.starts_with('$') {
                        ReferenceTarget::Dynamic
                    } else {
                        snapshot_target_for_name(
                            snapshot,
                            source_scope,
                            written,
                            ImportKind::Function,
                        )
                    };
                    add_reference(
                        builder,
                        file,
                        function.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::FunctionCall,
                        target,
                    );
                }
            }
            "member_call_expression" | "nullsafe_member_call_expression" => {
                if let (Some(object), Some(name)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("name"),
                ) {
                    let target = if name.kind() == "variable_name" {
                        ReferenceTarget::Dynamic
                    } else {
                        expression_member_target(
                            snapshot,
                            source_scope,
                            object,
                            node_text(name, text),
                            MemberKind::Method,
                            MemberAccess::Instance,
                            text,
                        )
                    };
                    add_reference(
                        builder,
                        file,
                        name.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::MethodCall,
                        target,
                    );
                }
            }
            "member_access_expression" | "nullsafe_member_access_expression" => {
                if let (Some(object), Some(name)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("name"),
                ) {
                    let role = if node.parent().is_some_and(|parent| {
                        parent.kind() == "assignment_expression"
                            && parent
                                .child_by_field_name("left")
                                .is_some_and(|left| left.byte_range().contains(&node.start_byte()))
                    }) {
                        ReferenceRole::PropertyWrite
                    } else {
                        ReferenceRole::PropertyRead
                    };
                    let target = expression_member_target(
                        snapshot,
                        source_scope,
                        object,
                        node_text(name, text),
                        MemberKind::Property,
                        MemberAccess::Instance,
                        text,
                    );
                    add_reference(
                        builder,
                        file,
                        name.byte_range(),
                        source_scope,
                        source_symbol,
                        role,
                        target,
                    );
                }
            }
            "static_call_expression" => {
                if let Some((class, name)) = static_member_parts(node) {
                    let target = static_member_target(
                        snapshot,
                        source_scope,
                        class,
                        node_text(name, text),
                        MemberKind::Method,
                        text,
                    );
                    add_reference(
                        builder,
                        file,
                        name.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::StaticMethodCall,
                        target,
                    );
                }
            }
            "class_constant_access_expression" => {
                if let Some((class, name)) = static_member_parts(node) {
                    let target = static_member_target(
                        snapshot,
                        source_scope,
                        class,
                        node_text(name, text),
                        MemberKind::ClassConstant,
                        text,
                    );
                    add_reference(
                        builder,
                        file,
                        name.byte_range(),
                        source_scope,
                        source_symbol,
                        ReferenceRole::ClassConstantRead,
                        target,
                    );
                }
            }
            "simple_parameter" => {
                if let Some(type_node) = node.child_by_field_name("type") {
                    add_type_references(
                        snapshot,
                        builder,
                        file,
                        source_scope,
                        source_symbol,
                        type_node,
                        ReferenceRole::ParameterType,
                        text,
                    );
                }
            }
            "function_definition"
            | "method_declaration"
            | "anonymous_function"
            | "arrow_function" => {
                if let Some(type_node) = node.child_by_field_name("return_type") {
                    add_type_references(
                        snapshot,
                        builder,
                        file,
                        source_scope,
                        source_symbol,
                        type_node,
                        ReferenceRole::ReturnType,
                        text,
                    );
                }
            }
            "property_declaration" => {
                if let Some(type_node) = node.child_by_field_name("type") {
                    add_type_references(
                        snapshot,
                        builder,
                        file,
                        source_scope,
                        source_symbol,
                        type_node,
                        ReferenceRole::Type,
                        text,
                    );
                }
            }
            "echo_statement" => {
                for child in node.named_children(&mut node.walk()) {
                    if matches!(child.kind(), "name" | "qualified_name") {
                        add_reference(
                            builder,
                            file,
                            child.byte_range(),
                            source_scope,
                            source_symbol,
                            ReferenceRole::GlobalConstantRead,
                            snapshot_target_for_name(
                                snapshot,
                                source_scope,
                                node_text(child, text),
                                ImportKind::Constant,
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
        stack.extend(node.named_children(&mut node.walk()));
    }
}

fn source_symbol_for_scope(
    snapshot: &SemanticSnapshot,
    file: FileId,
    scope: &Scope,
) -> Option<SymbolId> {
    let expected_kind = match scope.kind {
        ScopeKind::Function | ScopeKind::Closure | ScopeKind::ArrowFunction => {
            ProjectSymbolKind::Function
        }
        ScopeKind::Method => ProjectSymbolKind::Method,
        _ => return None,
    };
    snapshot
        .symbols_for_file(file)
        .iter()
        .filter_map(|id| snapshot.symbol(*id))
        .filter(|symbol| {
            symbol.kind == expected_kind
                && symbol.range.start >= scope.span.start
                && symbol.range.start < scope.span.end
        })
        .min_by_key(|symbol| symbol.range.start)
        .map(|symbol| symbol.id)
}

fn static_member_parts(
    node: tree_sitter::Node<'_>,
) -> Option<(tree_sitter::Node<'_>, tree_sitter::Node<'_>)> {
    let class = node
        .child_by_field_name("class")
        .or_else(|| node.child_by_field_name("object"));
    let name = node.child_by_field_name("name");
    if let (Some(class), Some(name)) = (class, name) {
        return Some((class, name));
    }
    let children: Vec<_> = node.named_children(&mut node.walk()).collect();
    Some((*children.first()?, *children.get(1)?))
}

fn object_creation_class(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    node.child_by_field_name("class")
        .or_else(|| node.named_children(&mut node.walk()).next())
}

fn add_type_references(
    snapshot: &SemanticSnapshot,
    builder: &mut SnapshotBuilder,
    file: FileId,
    scope: ScopeId,
    source_symbol: Option<SymbolId>,
    node: tree_sitter::Node<'_>,
    role: ReferenceRole,
    text: &str,
) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if matches!(current.kind(), "name" | "qualified_name")
            && current
                .parent()
                .is_none_or(|parent| parent.kind() != "qualified_name")
        {
            add_reference(
                builder,
                file,
                current.byte_range(),
                scope,
                source_symbol,
                role,
                snapshot_target_for_name(
                    snapshot,
                    scope,
                    node_text(current, text),
                    ImportKind::Class,
                ),
            );
        }
        stack.extend(current.named_children(&mut current.walk()));
    }
}

fn add_named_type_references(
    snapshot: &SemanticSnapshot,
    builder: &mut SnapshotBuilder,
    file: FileId,
    scope: ScopeId,
    source_symbol: Option<SymbolId>,
    node: tree_sitter::Node<'_>,
    role: ReferenceRole,
    text: &str,
) {
    for child in node.named_children(&mut node.walk()) {
        if matches!(child.kind(), "name" | "qualified_name" | "relative_name") {
            add_reference(
                builder,
                file,
                child.byte_range(),
                scope,
                source_symbol,
                role,
                snapshot_target_for_name(
                    snapshot,
                    scope,
                    node_text(child, text),
                    ImportKind::Class,
                ),
            );
        }
    }
}

fn add_import_references(
    snapshot: &SemanticSnapshot,
    builder: &mut SnapshotBuilder,
    file: FileId,
    scope: ScopeId,
    source_symbol: Option<SymbolId>,
    node: tree_sitter::Node<'_>,
    text: &str,
) {
    let raw = node_text(node, text);
    let start = node.start_byte();
    let Some(current) = snapshot.scope(scope) else {
        return;
    };
    for (kind, bindings) in [
        (ImportKind::Class, &current.imports.classes),
        (ImportKind::Function, &current.imports.functions),
        (ImportKind::Constant, &current.imports.constants),
    ] {
        for binding in bindings.values() {
            let Some(local) = raw.find(&binding.alias) else {
                continue;
            };
            // An alias may occur in a parameter or string in malformed code;
            // constrain the hit to the import statement's name area.
            let span = (start + local)..(start + local + binding.alias.len());
            add_reference(
                builder,
                file,
                span,
                scope,
                source_symbol,
                ReferenceRole::Import,
                snapshot_target_for_name(snapshot, scope, &binding.alias, kind),
            );
        }
    }
}

fn add_reference(
    builder: &mut SnapshotBuilder,
    file: FileId,
    span: std::ops::Range<usize>,
    source_scope: ScopeId,
    source_symbol: Option<SymbolId>,
    role: ReferenceRole,
    target: ReferenceTarget,
) {
    let id = ReferenceId(builder.references.records.len() as u32);
    let reference = SemanticReference {
        id,
        file,
        span,
        source_scope,
        source_symbol,
        role,
        target: target.clone(),
    };
    builder.references.records.push(reference);
    builder
        .references
        .references_by_file
        .entry(file)
        .or_default()
        .push(id);
    match target {
        ReferenceTarget::Resolved(symbol) => builder
            .references
            .references_by_target
            .entry(symbol)
            .or_default()
            .push(id),
        ReferenceTarget::Candidates(_) => builder.references.ambiguous_references.push(id),
        ReferenceTarget::Unresolved | ReferenceTarget::Dynamic | ReferenceTarget::Deferred => {}
    }
}

fn snapshot_target_for_name(
    snapshot: &SemanticSnapshot,
    scope: ScopeId,
    written: &str,
    kind: ImportKind,
) -> ReferenceTarget {
    let Some(resolved) = snapshot.resolve_name(scope, written, kind) else {
        return ReferenceTarget::Unresolved;
    };
    let ids: Vec<_> = snapshot
        .symbols_for_fqn(&resolved)
        .iter()
        .copied()
        .filter(|id| {
            snapshot.symbol(*id).is_some_and(|symbol| match kind {
                ImportKind::Class => matches!(
                    symbol.kind,
                    ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum
                ),
                ImportKind::Function => symbol.kind == ProjectSymbolKind::Function,
                ImportKind::Constant => {
                    matches!(
                        symbol.kind,
                        ProjectSymbolKind::Constant | ProjectSymbolKind::ClassConstant
                    )
                }
            })
        })
        .collect();
    target_from_ids(ids, written)
}

fn target_from_ids(ids: Vec<SymbolId>, written: &str) -> ReferenceTarget {
    match ids.as_slice() {
        [] if written.contains('\\') => ReferenceTarget::Deferred,
        [] => ReferenceTarget::Unresolved,
        [id] => ReferenceTarget::Resolved(*id),
        _ => ReferenceTarget::Candidates(ids),
    }
}

fn expression_member_target(
    snapshot: &SemanticSnapshot,
    scope: ScopeId,
    object: tree_sitter::Node<'_>,
    name: &str,
    kind: MemberKind,
    access: MemberAccess,
    text: &str,
) -> ReferenceTarget {
    let Some(expression) = expression_from_ast(object, text) else {
        return if node_text(object, text).contains('$') {
            ReferenceTarget::Dynamic
        } else {
            ReferenceTarget::Unresolved
        };
    };
    let Some(receiver) =
        ExpressionResolver::new(snapshot, scope).infer_expression_type(&expression)
    else {
        return ReferenceTarget::Unresolved;
    };
    member_target(snapshot, scope, receiver, name, kind, access)
}

fn static_member_target(
    snapshot: &SemanticSnapshot,
    scope: ScopeId,
    class: tree_sitter::Node<'_>,
    name: &str,
    kind: MemberKind,
    text: &str,
) -> ReferenceTarget {
    let Some(resolved) = snapshot.resolve_class_name(scope, node_text(class, text)) else {
        return ReferenceTarget::Unresolved;
    };
    let receiver = DeclaredType::Named {
        written: node_text(class, text).to_owned(),
        resolved,
    };
    member_target(snapshot, scope, receiver, name, kind, MemberAccess::Static)
}

fn member_target(
    snapshot: &SemanticSnapshot,
    scope: ScopeId,
    receiver: DeclaredType,
    name: &str,
    kind: MemberKind,
    access: MemberAccess,
) -> ReferenceTarget {
    match match kind {
        MemberKind::Method => snapshot
            .member_resolver()
            .resolve_method(scope, &receiver, name, access),
        MemberKind::Property => snapshot
            .member_resolver()
            .resolve_property(scope, &receiver, name),
        MemberKind::ClassConstant => snapshot
            .member_resolver()
            .resolve_class_constant(scope, &receiver, name),
    } {
        MemberResolution::Resolved(id)
        | MemberResolution::ResolvedButInaccessible(id)
        | MemberResolution::Incompatible(id) => ReferenceTarget::Resolved(id),
        MemberResolution::Candidates(ids) => ReferenceTarget::Candidates(ids),
        MemberResolution::Deferred(_) => ReferenceTarget::Deferred,
        MemberResolution::Unresolved => ReferenceTarget::Unresolved,
    }
}

fn innermost_callable_scope(
    builder: &SnapshotBuilder,
    file: FileId,
    offset: usize,
) -> Option<ScopeId> {
    builder
        .scopes
        .records
        .iter()
        .filter(|scope| {
            matches!(
                scope.kind,
                ScopeKind::Function
                    | ScopeKind::Method
                    | ScopeKind::Closure
                    | ScopeKind::ArrowFunction
            ) && scope.file == Some(file)
                && scope.span.contains(&offset)
        })
        .max_by_key(|scope| scope.span.start)
        .map(|scope| scope.id)
}

/// Selects the lexical scope that owns an assignment. Callable scopes take
/// precedence; assignments at namespace/file level are attached to the most
/// specific Namespace scope, falling back to the file scope.
fn assignment_scope(builder: &SnapshotBuilder, file: FileId, offset: usize) -> Option<ScopeId> {
    innermost_callable_scope(builder, file, offset).or_else(|| {
        let namespaces = builder
            .scopes
            .records
            .iter()
            .filter(|scope| {
                scope.file == Some(file)
                    && scope.kind == ScopeKind::Namespace
                    && scope.span.contains(&offset)
            });
        namespaces
            .max_by_key(|scope| scope.span.start)
            .or_else(|| {
                // Tree-sitter represents unbracketed namespace bodies with a
                // declaration span that may end before the body. In that
                // form, the active namespace is the latest declaration before
                // the assignment offset.
                builder
                    .scopes
                    .records
                    .iter()
                    .filter(|scope| {
                        scope.file == Some(file)
                            && scope.kind == ScopeKind::Namespace
                            && scope.span.start <= offset
                    })
                    .max_by_key(|scope| scope.span.start)
            })
            .map(|scope| scope.id)
    }).or_else(|| {
        builder
            .scopes
            .records
            .iter()
            .find(|scope| {
                scope.file == Some(file)
                    && scope.kind == ScopeKind::File
                    && scope.span.contains(&offset)
            })
            .map(|scope| scope.id)
    })
}

fn nearest_namespace(builder: &SnapshotBuilder, mut scope: ScopeId) -> String {
    loop {
        let Some(current) = builder.scopes.records.get(scope.0 as usize) else {
            return String::new();
        };
        if current.kind == ScopeKind::Namespace {
            return current.namespace.clone();
        }
        scope = match current.parent {
            Some(parent) => parent,
            None => return current.namespace.clone(),
        };
    }
}

fn nearest_class(builder: &SnapshotBuilder, mut scope: ScopeId) -> Option<&Scope> {
    loop {
        let current = builder.scopes.records.get(scope.0 as usize)?;
        if matches!(
            current.kind,
            ScopeKind::Class | ScopeKind::Interface | ScopeKind::Trait | ScopeKind::Enum
        ) {
            return Some(current);
        }
        scope = current.parent?;
    }
}

fn add_parameters(
    builder: &mut SnapshotBuilder,
    scope: ScopeId,
    parameters: Option<tree_sitter::Node<'_>>,
    text: &str,
) {
    let Some(parameters) = parameters else {
        return;
    };
    let mut stack = vec![parameters];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "simple_parameter" | "variadic_parameter" | "property_promotion_parameter"
        ) {
            let Some(name_node) = node.child_by_field_name("name") else {
                continue;
            };
            let name = node_text(name_node, text).trim().to_owned();
            if !name.starts_with('$') {
                continue;
            }
            let type_text = node
                .child_by_field_name("type")
                .map(|ty| node_text(ty, text).trim().to_owned());
            let declared_type = type_text
                .as_deref()
                .map(|ty| declared_type(ty, builder, scope));
            builder.scopes.records[scope.0 as usize]
                .bindings
                .push(VariableBinding {
                    name,
                    declaration_span: node.byte_range(),
                    declared_type,
                });
            continue;
        }
        stack.extend(node.named_children(&mut node.walk()));
    }
}

fn declared_type(raw: &str, builder: &SnapshotBuilder, scope: ScopeId) -> DeclaredType {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('?') {
        return DeclaredType::Nullable(Box::new(declared_type(inner, builder, scope)));
    }
    if raw.contains('|') {
        return DeclaredType::Union(
            raw.split('|')
                .map(|part| declared_type(part, builder, scope))
                .collect(),
        );
    }
    if raw.contains('&') {
        return DeclaredType::Intersection(
            raw.split('&')
                .map(|part| declared_type(part, builder, scope))
                .collect(),
        );
    }
    let lower = raw.to_ascii_lowercase();
    let builtin = match lower.as_str() {
        "int" => Some(BuiltinType::Int),
        "string" => Some(BuiltinType::String),
        "bool" => Some(BuiltinType::Bool),
        "float" => Some(BuiltinType::Float),
        "array" => Some(BuiltinType::Array),
        "object" => Some(BuiltinType::Object),
        "callable" => Some(BuiltinType::Callable),
        "iterable" => Some(BuiltinType::Iterable),
        "mixed" => Some(BuiltinType::Mixed),
        "void" => Some(BuiltinType::Void),
        "never" => Some(BuiltinType::Never),
        "null" => Some(BuiltinType::Null),
        "false" => Some(BuiltinType::False),
        "true" => Some(BuiltinType::True),
        _ => None,
    };
    if let Some(builtin) = builtin {
        return DeclaredType::Builtin(builtin);
    }
    if matches!(lower.as_str(), "self" | "static" | "parent") {
        let resolved = nearest_class(builder, scope)
            .and_then(|class| {
                if lower == "parent" {
                    class.parent_class.clone()
                } else {
                    class.class_name.clone()
                }
            })
            .unwrap_or_else(|| raw.to_owned());
        return DeclaredType::Named {
            written: raw.to_owned(),
            resolved,
        };
    }
    let resolved = resolve_builder_name(builder, scope, raw, ImportKind::Class);
    DeclaredType::Named {
        written: raw.to_owned(),
        resolved,
    }
}

fn resolve_builder_name(
    builder: &SnapshotBuilder,
    mut scope: ScopeId,
    written: &str,
    kind: ImportKind,
) -> String {
    // FUTURE REFACTOR PLAN: `resolve_builder_name` and
    // `SemanticSnapshot::resolve_name` intentionally mirror one another today.
    // Before changing either implementation, introduce a read-only
    // `NameResolutionContext` trait exposing `scope`, imports and namespace;
    // implement it for `SnapshotBuilder` and `SemanticSnapshot`, then make
    // `declared_type` and `declared_type_from_snapshot` share one parser. This
    // keeps build-time and post-build resolution behavior identical without a
    // broad semantic-index rewrite in this milestone.
    let written = written.trim();
    let absolute = written.starts_with('\\');
    let written = written.trim_start_matches('\\');
    let first = written.split('\\').next().unwrap_or(written);
    let mut namespace = String::new();
    loop {
        let Some(current) = builder.scopes.records.get(scope.0 as usize) else {
            break;
        };
        if let Some(binding) = current.imports.get(kind, first) {
            let suffix = written
                .split_once('\\')
                .map(|(_, tail)| format!("\\{tail}"))
                .unwrap_or_default();
            return format!("{}{}", binding.target, suffix);
        }
        if !current.namespace.is_empty() {
            namespace = current.namespace.clone();
        }
        scope = match current.parent {
            Some(parent) => parent,
            None => break,
        };
    }
    if absolute || namespace.is_empty() {
        written.to_owned()
    } else {
        format!("{namespace}\\{written}")
    }
}

fn parse_imports(raw: &str, table: &mut ImportTable) {
    let mut value = raw
        .trim()
        .trim_start_matches("use ")
        .trim()
        .trim_end_matches(';')
        .trim();
    let kind = if let Some(rest) = value.strip_prefix("function ") {
        value = rest.trim();
        ImportKind::Function
    } else if let Some(rest) = value.strip_prefix("const ") {
        value = rest.trim();
        ImportKind::Constant
    } else {
        ImportKind::Class
    };
    if let Some((prefix, body)) = value.split_once('{') {
        let prefix = prefix.trim().trim_end_matches('\\');
        let body = body.trim_end_matches('}').trim();
        for item in body.split(',') {
            add_import(item.trim(), prefix, kind, table);
        }
    } else {
        for item in value.split(',') {
            add_import(item.trim(), "", kind, table);
        }
    }
}

fn add_import(item: &str, prefix: &str, kind: ImportKind, table: &mut ImportTable) {
    if item.is_empty() {
        return;
    }
    let (name, alias) = item
        .split_once(" as ")
        .map(|(name, alias)| (name.trim(), alias.trim().to_owned()))
        .unwrap_or((
            item,
            item.rsplit('\\').next().unwrap_or(item).trim().to_owned(),
        ));
    let target = if prefix.is_empty() {
        name.trim_start_matches('\\').to_owned()
    } else {
        format!(
            "{}\\{}",
            prefix.trim_start_matches('\\'),
            name.trim_start_matches('\\')
        )
    };
    table.insert(ImportBinding {
        alias,
        target,
        kind,
    });
}

fn normalize_path(path: &Path) -> String {
    let mut lexical = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                lexical.pop();
            }
            _ => lexical.push(component.as_os_str()),
        }
    }
    // Do not switch between lexical and canonical forms based on whether the
    // file currently exists. Buffers are often indexed before their first
    // save, and that transition must not change the persistent identity.
    let normalized = lexical.to_string_lossy().replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    // Windows extended UNC (`\\?\\UNC\\host\\...`) and ordinary UNC
    // (`\\host\\...`) identify the same file.  Collapse both spellings to
    // the stable ordinary UNC form before using the path as a persistent key.
    if lower.starts_with("//?/unc/") {
        return format!("//{}", &normalized[8..]);
    }
    if lower.starts_with("//?/") {
        return normalized[4..].to_owned();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Instant};

    #[test]
    fn workspace_physical_identity_uses_canonical_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("src/A.php");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "<?php").unwrap();
        let dotted = dir.path().join("src/../src/A.php");
        assert_eq!(
            PersistentFileKey::workspace_physical(&real),
            PersistentFileKey::workspace_physical(&dotted)
        );
        assert_eq!(
            PersistentFileKey::workspace_physical_with_root("src/A.php", dir.path()),
            PersistentFileKey::workspace_physical(&real)
        );
    }

    #[test]
    fn workspace_lexical_is_stable_without_filesystem_access() {
        let path = Path::new("missing/../src/Unsaved.php");
        let first = PersistentFileKey::workspace_lexical(path);
        let second = PersistentFileKey::workspace_lexical(path);
        assert_eq!(first, second);
        assert_eq!(first.origin, SourceOrigin::Workspace);
        assert_eq!(first.normalized_path, "src/Unsaved.php");
    }

    #[test]
    fn workspace_lexical_matches_physical_index_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("src/User.php");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "<?php").unwrap();
        let canonical = std::fs::canonicalize(&path).unwrap();
        assert_eq!(
            PersistentFileKey::workspace_lexical(&canonical),
            PersistentFileKey::workspace_physical(&path)
        );
    }

    #[test]
    fn scope_lookup_falls_back_to_resident_file_at_eof_without_fabricating_bindings() {
        let text = "<?php namespace App; function run() { return 1; }";
        let mut builder = SnapshotBuilder::from_php_text("scope.php", text, SemanticRevision(1));
        let file_scope = builder
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::File)
            .map(|scope| scope.id)
            .unwrap();
        builder.scopes.records[file_scope.0 as usize].span = 0..text.len() - 1;
        let snapshot = builder.build();
        let at_end = snapshot
            .scope_id_at_file(FileId(0), text.len())
            .expect("file fallback at exact EOF");
        assert!(matches!(
            snapshot.scopes.records[at_end.0 as usize].kind,
            ScopeKind::Namespace | ScopeKind::File
        ));
        let beyond = snapshot
            .scope_id_at_file(FileId(0), text.len() + 10)
            .expect("file fallback beyond stale end");
        assert_eq!(beyond, at_end);
        assert!(snapshot.scope_id_at_file(FileId(99), text.len()).is_none());
        assert!(snapshot.lookup_binding(at_end, "$missing").is_none());

        let inside = text.find("return").unwrap();
        let inner = snapshot.scope_id_at_file(FileId(0), inside).unwrap();
        assert_eq!(snapshot.scopes.records[inner.0 as usize].kind, ScopeKind::Function);
    }

    #[test]
    fn persistent_file_key_collapses_extended_unc_spelling() {
        let ordinary =
            PersistentFileKey::workspace(Path::new("//wsl.localhost/Ubuntu/home/foo/bar.php"));
        let extended = PersistentFileKey::workspace(Path::new(
            "//?/UNC/wsl.localhost/Ubuntu/home/foo/bar.php",
        ));
        assert_eq!(ordinary, extended);
    }

    fn fixture_index() -> (tempfile::TempDir, ProjectSymbolIndex) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("UserService.php");
        fs::write(
            &path,
            "<?php\nnamespace App\\Services;\nclass UserService\n{\n    public function findUser() {}\n    public function saveUser() {}\n}\n",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        (dir, index)
    }

    #[test]
    fn empty_snapshot_has_revision_and_no_declarations() {
        let snapshot = SemanticSnapshot::empty(SemanticRevision(7));
        assert_eq!(snapshot.revision, SemanticRevision(7));
        assert!(snapshot.files.records.is_empty());
        assert!(snapshot.symbols.records.is_empty());
    }

    #[test]
    fn scope_compaction_trigger_obeys_ratio_minimum_and_empty_edges() {
        assert!(!should_compact_scopes(20, 5));
        assert!(!should_compact_scopes(255, 1));
        assert!(should_compact_scopes(256, 32));
        assert!(!should_compact_scopes(256, 33));
        assert!(!should_compact_scopes(20, 20));
        assert!(should_compact_scopes(256, 0));
        assert!(!should_compact_scopes(0, 0));
    }

    #[test]
    fn replacing_target_file_invalidates_cross_file_reference_to_removed_symbol() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.php");
        let b = dir.path().join("B.php");
        fs::write(
            &a,
            "<?php namespace App; class Service { public function run(): void {} }",
        )
        .unwrap();
        fs::write(
            &b,
            "<?php namespace App; function test(Service $service): void { $service->run(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let old_run = base.symbols_for_fqn("App\\Service::run")[0];
        assert!(
            !base
                .find_usages(old_run, FindUsagesOptions::default())
                .usages
                .is_empty()
        );

        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.replace_workspace_file(
            &a,
            "<?php namespace App; class Service { public function execute(): void {} }",
        );
        let next = builder.finish();
        assert!(next.symbols_for_fqn("App\\Service::run").is_empty());
        assert!(next.symbols_for_fqn("App\\Service::execute").len() == 1);
        assert!(
            next.find_usages(old_run, FindUsagesOptions::default())
                .usages
                .is_empty()
        );
        let b_file = next.file_id(&PersistentFileKey::workspace(&b)).unwrap();
        assert!(
            next.references_for_file(b_file)
                .iter()
                .filter_map(|id| next.reference(*id))
                .all(|reference| {
                    !matches!(reference.target, ReferenceTarget::Resolved(id) if id == old_run)
                })
        );
    }

    #[test]
    #[ignore = "long-running scope growth audit; run with --ignored"]
    fn audit_repeated_replacements_append_detached_scopes_but_reuse_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("A.php");
        let initial = "<?php class A { public function run(): void {} }";
        fs::write(&path, initial).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let mut current = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let initial_symbols = current.symbols.records.len();
        let initial_scopes = current.scopes.records.len();
        for iteration in 0..100 {
            let mut builder = SnapshotBuilder::from_snapshot(&current);
            builder.replace_workspace_file(
                &path,
                format!("<?php // {iteration}\nclass A {{ public function run(): void {{}} }}"),
            );
            current = builder.finish();
        }
        assert_eq!(current.symbols.records.len(), initial_symbols);
        assert!(current.scopes.records.len() > initial_scopes);
        assert_eq!(current.references.records.len(), 0);
    }

    #[test]
    #[ignore = "long-running scope growth audit; run with --ignored"]
    fn audit_scope_growth_over_long_incremental_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Long.php");
        let base_text = "<?php namespace App; class Long { public function m01() {} public function m02() {} public function m03() {} public function m04() {} public function m05() {} public function m06() {} public function m07() {} public function m08() {} public function m09() {} public function m10() {} }";
        fs::write(&path, base_text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let mut current = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let file = current
            .file_id(&PersistentFileKey::workspace(&path))
            .unwrap();
        let json_path = dir.path().join("snapshot.json");
        current.save_json(&json_path).unwrap();
        println!(
            "[SCOPE AUDIT] iteration=0 scopes={} active_scopes={} symbols={} references={} snapshot_size_json_bytes={}",
            current.scopes.records.len(),
            current
                .scopes
                .records
                .iter()
                .filter(|scope| scope.file.is_some())
                .count(),
            current.symbols.records.len(),
            current.references.records.len(),
            fs::metadata(&json_path).unwrap().len(),
        );
        // 100 iterations is the lower bound of the requested audit and keeps
        // this diagnostic test practical in the normal test suite.
        for iteration in 1..=100 {
            let text = format!(
                "<?php namespace App; // iteration {iteration}\nclass Long {{ public function m01() {{ /* {iteration} */ }} public function m02() {{}} public function m03() {{}} public function m04() {{}} public function m05() {{}} public function m06() {{}} public function m07() {{}} public function m08() {{}} public function m09() {{}} public function m10() {{}} }}"
            );
            let offset = text.find("m01").unwrap();
            let mut builder = SnapshotBuilder::from_snapshot(&current);
            builder.replace_workspace_file(&path, text.clone());
            let t_finish = Instant::now();
            let next = builder.finish();
            let finish_us = t_finish.elapsed().as_micros();
            let t_scope = Instant::now();
            let _ = next.scope_at(file, offset);
            let scope_us = t_scope.elapsed().as_micros();
            let t_definition = Instant::now();
            let _ = next.definition_at_detailed(
                &path,
                &text,
                offset,
                DefinitionQueryContext {
                    document_version: None,
                    semantic_revision: next.revision,
                },
            );
            let definition_us = t_definition.elapsed().as_micros();
            let t_parent = Instant::now();
            let _ = next.member_resolver().parent_class_of("App\\Long");
            let parent_us = t_parent.elapsed().as_micros();
            let active_scopes = next
                .scopes
                .records
                .iter()
                .filter(|scope| scope.file.is_some())
                .count();
            if iteration % 100 == 0 {
                next.save_json(&json_path).unwrap();
                let json_bytes = fs::metadata(&json_path).unwrap().len();
                println!(
                    "[SCOPE AUDIT] iteration={iteration} scopes={} active_scopes={active_scopes} symbols={} references={} snapshot_size_json_bytes={json_bytes} scope_at_us={scope_us} definition_at_detailed_us={definition_us} parent_class_of_us={parent_us} finish_us={finish_us}",
                    next.scopes.records.len(),
                    next.symbols.records.len(),
                    next.references.records.len(),
                );
            }
            current = next;
        }
        let before_scopes = current.scopes.records.len();
        let active_scopes = current
            .scopes
            .records
            .iter()
            .filter(|scope| scope.file.is_some())
            .count();
        let symbols_before = current.symbols.records.clone();
        let json_before = fs::metadata(&json_path).unwrap().len();
        let mut compact_builder = SnapshotBuilder::from_snapshot(&current);
        compact_builder.compact_scopes();
        let compacted = compact_builder.finish();
        let json_compacted = dir.path().join("snapshot-compacted.json");
        compacted.save_json(&json_compacted).unwrap();
        assert_eq!(compacted.scopes.records.len(), active_scopes);
        assert!(
            compacted
                .scopes
                .records
                .iter()
                .enumerate()
                .all(|(index, scope)| scope.id == ScopeId(index as u32))
        );
        assert_eq!(compacted.symbols.records, symbols_before);
        assert_eq!(compacted.files.records.len(), current.files.records.len());
        assert!(
            compacted
                .files
                .records
                .iter()
                .zip(&current.files.records)
                .all(|(after, before)| after.id == before.id
                    && after.key == before.key
                    && after.path == before.path)
        );
        println!(
            "[SCOPE AUDIT] compacted total_scopes={} active_scopes={} snapshot_size_json_bytes={} before_scopes={} before_json_bytes={json_before}",
            compacted.scopes.records.len(),
            active_scopes,
            fs::metadata(&json_compacted).unwrap().len(),
            before_scopes,
        );
    }

    #[test]
    fn compact_scopes_preserves_parent_bindings_definition_and_usages() {
        let dir = tempfile::tempdir().unwrap();
        let service = dir.path().join("Service.php");
        let test = dir.path().join("test.php");
        let test_text = "<?php namespace App\\Test; use App\\Service; function test(Service $service): void { $closure = function() use ($service) {}; $service->run(); }";
        fs::write(
            &service,
            "<?php namespace App; class Service { public function run(): void {} }",
        )
        .unwrap();
        fs::write(&test, test_text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(9));
        let offset = test_text.rfind("run").unwrap();
        let context = DefinitionQueryContext {
            document_version: None,
            semantic_revision: base.revision,
        };
        let definition_before = base.definition_at(&test, test_text, offset, context);
        let usages_before = base.find_usages_at(&test, offset, FindUsagesOptions::default());
        let function_scope = base
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function && scope.file.is_some())
            .unwrap();
        assert_eq!(
            base.resolve_class_name(function_scope.id, "Service")
                .as_deref(),
            Some("App\\Service")
        );
        assert!(base.lookup_binding(function_scope.id, "$service").is_some());

        let changed_text = format!("// edit\n{test_text}");
        let changed_offset = changed_text.rfind("run").unwrap();
        let mut changed = SnapshotBuilder::from_snapshot(&base);
        changed.replace_workspace_file(&test, changed_text.clone());
        let changed = changed.finish();
        assert!(
            changed.scopes.records.len()
                > changed
                    .scopes
                    .records
                    .iter()
                    .filter(|scope| scope.file.is_some())
                    .count()
        );
        let active_kinds_before: Vec<_> = changed
            .scopes
            .records
            .iter()
            .filter(|scope| scope.file.is_some())
            .map(|scope| scope.kind)
            .collect();
        let mut compact_builder = SnapshotBuilder::from_snapshot(&changed);
        compact_builder.compact_scopes();
        let compacted = compact_builder.finish();
        let active_kinds_after: Vec<_> = compacted
            .scopes
            .records
            .iter()
            .map(|scope| scope.kind)
            .collect();
        assert_eq!(active_kinds_after, active_kinds_before);
        let changed_context = DefinitionQueryContext {
            document_version: None,
            semantic_revision: compacted.revision,
        };
        assert_eq!(
            compacted.definition_at(&test, &changed_text, changed_offset, changed_context),
            definition_before
        );
        let usages_after =
            compacted.find_usages_at(&test, changed_offset, FindUsagesOptions::default());
        assert_eq!(usages_after.status, usages_before.status);
        assert_eq!(usages_after.usages.len(), usages_before.usages.len());
        assert!(
            usages_after
                .usages
                .iter()
                .zip(&usages_before.usages)
                .all(|(after, before)| after.file == before.file && after.role == before.role)
        );
        let compact_function = compacted
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        assert_eq!(
            compacted
                .resolve_class_name(compact_function.id, "Service")
                .as_deref(),
            Some("App\\Service")
        );
        assert!(
            compacted
                .lookup_binding(compact_function.id, "$service")
                .is_some()
        );
        for reference in &compacted.references.records {
            assert!((reference.source_scope.0 as usize) < compacted.scopes.records.len());
        }
        assert!(
            base.scopes
                .records
                .iter()
                .all(|scope| (scope.id.0 as usize) < base.scopes.records.len())
        );
    }

    #[test]
    fn ids_are_compact_u32_values() {
        assert_eq!(std::mem::size_of::<FileId>(), 4);
        assert_eq!(std::mem::size_of::<SymbolId>(), 4);
    }

    #[test]
    fn snapshot_imports_project_declarations_and_owner_members() {
        let (_dir, index) = fixture_index();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let classes = snapshot.symbols_for_fqn("App\\Services\\UserService");
        assert_eq!(classes.len(), 1);
        let owner = snapshot.symbol(classes[0]).unwrap();
        assert_eq!(owner.name, "UserService");
        assert_eq!(owner.key.discriminator, None);
        let members = snapshot.members_of(owner.id);
        assert_eq!(members.len(), 2);
        assert!(
            members
                .iter()
                .any(|id| snapshot.symbol(*id).unwrap().name == "findUser")
        );
        assert!(
            members
                .iter()
                .any(|id| snapshot.symbol(*id).unwrap().name == "saveUser")
        );
        assert_eq!(snapshot.symbols_for_file(owner.file).len(), 3);
    }

    #[test]
    fn owner_index_contains_properties_and_class_constants() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("User.php"),
            "<?php namespace App; class User { public function save() {} private string $name; public const TYPE = 'user'; }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let owner = snapshot.symbols_for_fqn("App\\User")[0];
        let members = snapshot.members_of(owner);
        assert_eq!(members.len(), 3);
        assert!(
            members
                .iter()
                .all(|id| snapshot.symbol(*id).unwrap().owner == Some(owner))
        );
        assert!(
            members
                .iter()
                .any(|id| snapshot.symbol(*id).unwrap().name == "save")
        );
        assert!(
            members
                .iter()
                .any(|id| { snapshot.symbol(*id).unwrap().name.trim_start_matches('$') == "name" })
        );
        assert!(
            members
                .iter()
                .any(|id| snapshot.symbol(*id).unwrap().name == "TYPE")
        );
        assert_eq!(
            snapshot
                .symbols_for_file(snapshot.symbol(owner).unwrap().file)
                .len(),
            4
        );
    }

    #[test]
    fn same_member_name_in_different_namespaces_has_different_owners() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("A.php"),
            "<?php namespace A; class User { public function save() {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("B.php"),
            "<?php namespace B; class User { public function save() {} }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let a_owner = snapshot.symbols_for_fqn("A\\User")[0];
        let b_owner = snapshot.symbols_for_fqn("B\\User")[0];
        let a_save = snapshot.members_of(a_owner);
        let b_save = snapshot.members_of(b_owner);
        assert_eq!(a_save.len(), 1);
        assert_eq!(b_save.len(), 1);
        assert_ne!(a_owner, b_owner);
        assert_ne!(a_save[0], b_save[0]);
    }

    #[test]
    fn fqn_lookup_keeps_duplicate_candidates() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["A.php", "B.php"] {
            fs::write(
                dir.path().join(name),
                "<?php namespace App; class Duplicate {}",
            )
            .unwrap();
        }
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        assert_eq!(snapshot.symbols_for_fqn("App\\Duplicate").len(), 2);
    }

    #[test]
    fn duplicate_classes_in_one_file_are_preserved_without_arbitrary_membership() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Duplicate.php"),
            "<?php namespace App; class Duplicate { public function save() {} } class Duplicate { public function save() {} }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let owners = snapshot.symbols_for_fqn("App\\Duplicate");
        assert_eq!(owners.len(), 2);
        assert!(
            snapshot
                .symbol(owners[0])
                .unwrap()
                .key
                .discriminator
                .is_some()
        );
        assert!(
            snapshot
                .symbol(owners[1])
                .unwrap()
                .key
                .discriminator
                .is_some()
        );
        assert!(snapshot.members_of(owners[0]).is_empty());
        assert!(snapshot.members_of(owners[1]).is_empty());
        assert_eq!(
            snapshot
                .symbols
                .records
                .iter()
                .filter(|symbol| symbol.name == "save")
                .count(),
            2
        );
    }

    #[test]
    fn persistent_symbol_key_ignores_offset_changes_but_tracks_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("UserService.php");
        fs::write(
            &path,
            "<?php\nnamespace App\\Services;\nclass UserService { public function findUser() {} public function saveUser() {} }",
        )
        .unwrap();
        let mut first_index = ProjectSymbolIndex::new();
        first_index.index_project(dir.path()).unwrap();
        let first = SemanticSnapshot::from_project_index(&first_index, SemanticRevision(1));
        let first_key = first
            .symbols
            .records
            .iter()
            .find(|s| s.name == "findUser")
            .unwrap()
            .key
            .clone();

        fs::write(
            &path,
            "<?php\n\nnamespace App\\Services;\nclass UserService { public function findUser() {} public function saveUser() {} }",
        )
        .unwrap();
        let mut changed_index = ProjectSymbolIndex::new();
        changed_index.index_project(dir.path()).unwrap();
        let changed = SemanticSnapshot::from_project_index(&changed_index, SemanticRevision(2));
        let changed_key = changed
            .symbols
            .records
            .iter()
            .find(|s| s.name == "findUser")
            .unwrap()
            .key
            .clone();
        assert_eq!(first_key, changed_key);

        fs::write(
            &path,
            "<?php\nnamespace App\\Services;\nclass UserService { public function renamed() {} public function saveUser() {} }",
        )
        .unwrap();
        let mut renamed_index = ProjectSymbolIndex::new();
        renamed_index.index_project(dir.path()).unwrap();
        let renamed = SemanticSnapshot::from_project_index(&renamed_index, SemanticRevision(3));
        let renamed_key = renamed
            .symbols
            .records
            .iter()
            .find(|s| s.name == "renamed")
            .unwrap()
            .key
            .clone();
        assert_ne!(first_key, renamed_key);
    }

    #[test]
    fn unrelated_declarations_do_not_change_existing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("User.php");
        fs::write(
            &path,
            "<?php namespace App; class User { public function save() {} }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let before = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let before_keys: HashMap<_, _> = before
            .symbols
            .records
            .iter()
            .map(|symbol| (symbol.name.clone(), symbol.key.clone()))
            .collect();

        fs::write(
            &path,
            "<?php namespace App; class NewClass {} class User { public function save() {} public function extra() {} }",
        )
        .unwrap();
        let mut changed_index = ProjectSymbolIndex::new();
        changed_index.index_project(dir.path()).unwrap();
        let changed = SemanticSnapshot::from_project_index(&changed_index, SemanticRevision(2));
        for name in ["User", "save"] {
            let current = changed
                .symbols
                .records
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap();
            assert_eq!(before_keys.get(name), Some(&current.key));
        }

        fs::write(
            &path,
            "<?php namespace App; class User { public function save() {} }",
        )
        .unwrap();
        let mut removed_index = ProjectSymbolIndex::new();
        removed_index.index_project(dir.path()).unwrap();
        let removed = SemanticSnapshot::from_project_index(&removed_index, SemanticRevision(3));
        for name in ["User", "save"] {
            let current = removed
                .symbols
                .records
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap();
            assert_eq!(before_keys.get(name), Some(&current.key));
        }
    }

    #[test]
    fn changing_only_a_method_body_keeps_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("User.php");
        fs::write(
            &path,
            "<?php namespace App; class User { public function save() { return 1; } }",
        )
        .unwrap();
        let mut first_index = ProjectSymbolIndex::new();
        first_index.index_project(dir.path()).unwrap();
        let first = SemanticSnapshot::from_project_index(&first_index, SemanticRevision(1));
        let first_key = first
            .symbols
            .records
            .iter()
            .find(|symbol| symbol.name == "save")
            .unwrap()
            .key
            .clone();
        fs::write(
            &path,
            "<?php namespace App; class User { public function save() { return 2; } }",
        )
        .unwrap();
        let mut second_index = ProjectSymbolIndex::new();
        second_index.index_project(dir.path()).unwrap();
        let second = SemanticSnapshot::from_project_index(&second_index, SemanticRevision(2));
        let second_key = second
            .symbols
            .records
            .iter()
            .find(|symbol| symbol.name == "save")
            .unwrap()
            .key
            .clone();
        assert_eq!(first_key, second_key);
    }

    #[test]
    fn snapshots_coexist_without_mutating_the_old_one() {
        let (_dir, index) = fixture_index();
        let first = Arc::new(SemanticSnapshot::from_project_index(
            &index,
            SemanticRevision(1),
        ));
        let second = Arc::new(SemanticSnapshot::from_project_index(
            &index,
            SemanticRevision(2),
        ));
        assert_eq!(first.revision, SemanticRevision(1));
        assert_eq!(second.revision, SemanticRevision(2));
        assert_eq!(first.symbols.records.len(), second.symbols.records.len());
        assert_eq!(first.symbols_for_fqn("App\\Services\\UserService").len(), 1);
    }

    #[test]
    fn publishing_a_new_snapshot_does_not_mutate_the_old_snapshot() {
        let (_dir, index) = fixture_index();
        let engine = SemanticEngine::new();
        let first = Arc::new(SemanticSnapshot::from_project_index(
            &index,
            SemanticRevision(1),
        ));
        assert!(engine.publish(first.clone()));
        let old_symbols = first.symbols.records.clone();
        let old_by_fqn = first.symbols.by_fqn.clone();
        let second = Arc::new(SemanticSnapshot::from_project_index(
            &index,
            SemanticRevision(2),
        ));
        assert!(engine.publish(second.clone()));
        assert_eq!(first.symbols.records, old_symbols);
        assert_eq!(first.symbols.by_fqn, old_by_fqn);
        assert_eq!(first.symbols_for_fqn("App\\Services\\UserService").len(), 1);
        assert_eq!(
            second.symbols_for_fqn("App\\Services\\UserService").len(),
            1
        );
    }

    #[test]
    fn engine_publishes_only_monotonic_revisions_and_rejects_stale_bases() {
        let engine = SemanticEngine::new();
        let revision_one = Arc::new(SemanticSnapshot::empty(SemanticRevision(1)));
        assert!(engine.publish(revision_one));
        assert!(!engine.publish(Arc::new(SemanticSnapshot::empty(SemanticRevision(1)))));
        assert!(!engine.publish_from(
            SemanticRevision(0),
            Arc::new(SemanticSnapshot::empty(SemanticRevision(2))),
        ));
        assert!(engine.publish_from(
            SemanticRevision(1),
            Arc::new(SemanticSnapshot::empty(SemanticRevision(2))),
        ));
        assert_eq!(engine.snapshot().revision, SemanticRevision(2));
    }

    #[test]
    fn snapshot_persistence_round_trip_preserves_keys_and_indexes() {
        let (_dir, index) = fixture_index();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(3));
        let cache = tempfile::tempdir().unwrap().path().join("semantic.json");
        snapshot.save_json(&cache).unwrap();
        let restored = SemanticSnapshot::load_json(&cache).unwrap();
        assert_eq!(restored.revision, snapshot.revision);
        assert_eq!(restored.files.by_key, snapshot.files.by_key);
        assert_eq!(restored.symbols.by_key, snapshot.symbols.by_key);
        assert_eq!(
            restored.symbols_for_fqn("App\\Services\\UserService"),
            snapshot.symbols_for_fqn("App\\Services\\UserService")
        );
        for (key, id) in &snapshot.files.by_key {
            assert_eq!(restored.file_id(key), Some(*id));
        }
        for (key, id) in &snapshot.symbols.by_key {
            assert_eq!(restored.symbol_id(key), Some(*id));
        }
        let owner = restored.symbols_for_fqn("App\\Services\\UserService")[0];
        assert_eq!(restored.members_of(owner).len(), 2);
    }

    #[test]
    fn duplicate_candidates_survive_persistence() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("A.php"),
            "<?php namespace App; class User {} ",
        )
        .unwrap();
        fs::write(
            dir.path().join("B.php"),
            "<?php namespace App; class User {} ",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let cache = tempfile::tempdir().unwrap().path().join("semantic.json");
        snapshot.save_json(&cache).unwrap();
        let restored = SemanticSnapshot::load_json(&cache).unwrap();
        assert_eq!(restored.symbols_for_fqn("App\\User").len(), 2);
    }

    #[test]
    fn future_fixture_builds_namespace_import_and_parameter_binding() {
        let text = r#"<?php
declare(strict_types=1);
namespace Omegaalfa\HttpClient\Http;
use InvalidArgumentException;
use Omegaalfa\FiberEventLoop\Future;
use Throwable;
function await(Future $future): mixed { return $future->await(); }
"#;
        let snapshot =
            SnapshotBuilder::from_php_text("functions.php", text, SemanticRevision(1)).build();
        let function_scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        assert_eq!(function_scope.namespace, "Omegaalfa\\HttpClient\\Http");
        let binding = snapshot
            .lookup_binding(function_scope.id, "$future")
            .unwrap();
        assert_eq!(
            binding.declared_type,
            Some(DeclaredType::Named {
                written: "Future".into(),
                resolved: "Omegaalfa\\FiberEventLoop\\Future".into()
            })
        );
    }

    #[test]
    fn scopes_are_nested_and_multiple_namespaces_keep_imports_local() {
        let text = r#"<?php
namespace A;
use Foo\One;
function a(One $x) {}
namespace B;
use Bar\One;
function b(One $x) {}
"#;
        let snapshot =
            SnapshotBuilder::from_php_text("multi.php", text, SemanticRevision(1)).build();
        let functions: Vec<_> = snapshot
            .scopes
            .records
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Function)
            .collect();
        assert_eq!(functions.len(), 2);
        assert_eq!(
            snapshot
                .lookup_binding(functions[0].id, "$x")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Named {
                written: "One".into(),
                resolved: "Foo\\One".into()
            })
        );
        assert_eq!(
            snapshot
                .lookup_binding(functions[1].id, "$x")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Named {
                written: "One".into(),
                resolved: "Bar\\One".into()
            })
        );
    }

    #[test]
    fn same_file_multiple_namespaces_preserve_workspace_members_and_inheritance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.php");
        let text = r#"<?php
namespace App\Base;
class A { public function run(): void {} protected function guarded(): void {} private function secret(): void {} }
namespace App\Middle;
use App\Base\A;
class B extends A { public function test(): void { parent::run(); $this->guarded(); $this->secret(); } }
namespace App\Final;
use App\Middle\B;
class C extends B {}
namespace App\Test;
use App\Final\C;
function test(C $c): void { $c->run(); }
"#;
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(60));
        let a = snapshot.symbols_for_fqn("App\\Base\\A")[0];
        assert_eq!(
            snapshot
                .members_named(a, "run", ProjectSymbolKind::Method)
                .len(),
            1
        );
        assert_eq!(snapshot.symbols_for_fqn("App\\Middle\\B").len(), 1);
        assert_eq!(snapshot.symbols_for_fqn("App\\Final\\C").len(), 1);
        let offset = text.rfind("$c->run").unwrap() + "$c->".len();
        let detailed = snapshot.definition_at_detailed(
            &path,
            text,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(60),
            },
        );
        assert!(matches!(detailed.result, DefinitionResult::Resolved(_)));
        assert_eq!(detailed.outcome, SemanticDefinitionOutcome::Resolved);
    }

    #[test]
    fn multi_namespace_alias_does_not_leak_into_local_class_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alias.php");
        let text = r#"<?php
namespace App\Base;
class A { public function run(): void {} }
namespace App\Alias;
use App\Base\A as ParentA;
class AliasChild extends ParentA {}
function testAlias(AliasChild $child): void { $child->run(); }
"#;
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        assert!(
            index
                .symbols()
                .iter()
                .any(|s| s.fully_qualified_name == "App\\Alias\\AliasChild")
        );
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(61));
        let offset = text.rfind("run").unwrap();
        let detailed = snapshot.definition_at_detailed(
            &path,
            text,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(61),
            },
        );
        assert!(matches!(detailed.result, DefinitionResult::Resolved(_)));
        assert_eq!(detailed.outcome, SemanticDefinitionOutcome::Resolved);
    }

    #[test]
    fn declared_types_cover_nullable_union_intersection_and_builtins() {
        let text = "<?php namespace App; function f(?Foo $a, Bar|Baz $b, A&B $c, int $d) {}";
        let snapshot =
            SnapshotBuilder::from_php_text("types.php", text, SemanticRevision(1)).build();
        let scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        assert!(matches!(
            snapshot
                .lookup_binding(scope.id, "$a")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Nullable(_))
        ));
        assert!(matches!(
            snapshot
                .lookup_binding(scope.id, "$b")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Union(_))
        ));
        assert!(matches!(
            snapshot
                .lookup_binding(scope.id, "$c")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Intersection(_))
        ));
        assert_eq!(
            snapshot
                .lookup_binding(scope.id, "$d")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Builtin(BuiltinType::Int))
        );
    }

    #[test]
    fn method_context_models_this_self_static_and_parent() {
        let text = "<?php namespace App; class ParentClass {} class Child extends ParentClass { public function save(self $v) {} public static function make() {} }";
        let snapshot =
            SnapshotBuilder::from_php_text("classes.php", text, SemanticRevision(1)).build();
        let methods: Vec<_> = snapshot
            .scopes
            .records
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Method)
            .collect();
        assert_eq!(methods.len(), 2);
        let instance = methods
            .iter()
            .find(|scope| !scope.is_static_method)
            .unwrap();
        assert!(snapshot.lookup_binding(instance.id, "$this").is_some());
        assert_eq!(
            snapshot.resolve_class_name(instance.id, "self").as_deref(),
            Some("App\\Child")
        );
        assert_eq!(
            snapshot
                .resolve_class_name(instance.id, "parent")
                .as_deref(),
            Some("App\\ParentClass")
        );
        let static_method = methods.iter().find(|scope| scope.is_static_method).unwrap();
        assert!(snapshot.lookup_binding(static_method.id, "$this").is_none());
        assert_eq!(
            snapshot
                .resolve_class_name(static_method.id, "static")
                .as_deref(),
            Some("App\\Child")
        );
    }

    #[test]
    fn imports_support_alias_group_function_and_constant_forms() {
        let text = "<?php namespace App; use Foo\\Bar as Baz; use Foo\\Group\\{One, Two as Alias}; use function Lib\\run; use const Lib\\FLAG; function f(Baz $x) {}";
        let snapshot =
            SnapshotBuilder::from_php_text("imports.php", text, SemanticRevision(1)).build();
        let namespace = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Namespace)
            .unwrap();
        assert_eq!(
            namespace.imports.classes.get("Baz").unwrap().target,
            "Foo\\Bar"
        );
        assert_eq!(
            namespace.imports.classes.get("Alias").unwrap().target,
            "Foo\\Group\\Two"
        );
        assert_eq!(
            namespace.imports.functions.get("run").unwrap().target,
            "Lib\\run"
        );
        assert_eq!(
            namespace.imports.constants.get("FLAG").unwrap().target,
            "Lib\\FLAG"
        );
    }

    #[test]
    fn closure_parameters_absolute_names_and_incomplete_php_are_safe() {
        let text = "<?php namespace App; $callback = function(Foo $value) {}; new \\Vendor\\Thing(); function broken(Foo $x";
        let snapshot =
            SnapshotBuilder::from_php_text("incomplete.php", text, SemanticRevision(1)).build();
        let closure = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Closure)
            .unwrap();
        assert!(snapshot.lookup_binding(closure.id, "$value").is_some());
        assert_eq!(
            snapshot
                .resolve_class_name(closure.id, "\\Vendor\\Thing")
                .as_deref(),
            Some("Vendor\\Thing")
        );
    }

    #[test]
    fn member_resolver_follows_typed_future_parameter_to_await() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/Future.php"),
            "<?php namespace Omegaalfa\\FiberEventLoop; class Future { public function await(): mixed {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/functions.php"),
            "<?php namespace Omegaalfa\\HttpClient\\Http; use Omegaalfa\\FiberEventLoop\\Future; function await(Future $future): mixed { return $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let function_scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        let result = snapshot.member_resolver().resolve_binding_method(
            function_scope.id,
            "$future",
            "await",
        );
        let target = match result {
            MemberResolution::Resolved(id) => snapshot.symbol(id).unwrap(),
            other => panic!("unexpected member resolution: {other:?}"),
        };
        assert_eq!(
            target.fully_qualified_name,
            "Omegaalfa\\FiberEventLoop\\Future::await"
        );
    }

    #[test]
    fn member_resolver_handles_property_constant_static_and_visibility() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("User.php"),
            "<?php namespace App; class User { public string $name; public const TYPE = 'user'; public static function create(): self {} private function secret(): void {} } function useUser(User $user) { $user->name; User::TYPE; User::create(); $user->secret(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        let user = snapshot
            .lookup_binding(scope.id, "$user")
            .unwrap()
            .declared_type
            .clone()
            .unwrap();
        assert!(matches!(
            snapshot
                .member_resolver()
                .resolve_property(scope.id, &user, "name"),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            snapshot
                .member_resolver()
                .resolve_class_constant(scope.id, &user, "TYPE"),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            snapshot.member_resolver().resolve_method(
                scope.id,
                &user,
                "create",
                MemberAccess::Static
            ),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            snapshot
                .member_resolver()
                .resolve_binding_method(scope.id, "$user", "secret"),
            MemberResolution::ResolvedButInaccessible(_)
        ));
    }

    #[test]
    fn member_resolver_distinguishes_static_and_instance_access() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("User.php"), "<?php namespace App; class User { public function save() {} public static function make() {} } function run(User $user) {} ").unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        let user = snapshot
            .lookup_binding(scope.id, "$user")
            .unwrap()
            .declared_type
            .clone()
            .unwrap();
        assert!(matches!(
            snapshot.member_resolver().resolve_method(
                scope.id,
                &user,
                "make",
                MemberAccess::Instance
            ),
            MemberResolution::Incompatible(_)
        ));
        assert!(matches!(
            snapshot.member_resolver().resolve_method(
                scope.id,
                &user,
                "save",
                MemberAccess::Static
            ),
            MemberResolution::Incompatible(_)
        ));
    }

    #[test]
    fn parameter_type_alias_resolves_member_without_short_name_ambiguity() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("one.php"),
            "<?php namespace App\\One; class StripeService { public function pay(): void {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("two.php"),
            "<?php namespace App\\Two; class StripeService { public function pay(): void {} }",
        )
        .unwrap();
        let test = "<?php namespace App\\Test; use App\\Two\\StripeService; function test(StripeService $service): void { $service->pay(); }";
        fs::write(dir.path().join("test.php"), test).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let offset = test.find("$service->pay").unwrap() + "$service->".len();
        let result = snapshot.definition_at_detailed(
            dir.path().join("test.php"),
            test,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(1),
            },
        );
        assert_eq!(
            result.outcome,
            SemanticDefinitionOutcome::Resolved,
            "{result:?}"
        );
        let DefinitionResult::Resolved(candidate) = result.result else {
            panic!("{result:?}")
        };
        assert!(candidate.location.file.ends_with("two.php"));
    }

    #[test]
    fn parameter_type_import_alias_resolves_member() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("service.php"),
            "<?php namespace App\\Services; class StripeService { public function pay(): void {} }",
        )
        .unwrap();
        let test = "<?php namespace App\\Test; use App\\Services\\StripeService as PaymentProcessor; function test(PaymentProcessor $service): void { $service->pay(); }";
        fs::write(dir.path().join("test.php"), test).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let offset = test.find("$service->pay").unwrap() + "$service->".len();
        let result = snapshot.definition_at_detailed(
            dir.path().join("test.php"),
            test,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(1),
            },
        );
        let DefinitionResult::Resolved(candidate) = result.result else {
            panic!("{result:?}")
        };
        assert!(candidate.location.file.ends_with("service.php"));
    }

    #[test]
    fn namespaced_typed_parameter_resolves_concrete_member() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("contract.php"), "<?php namespace App\\Contracts; interface PaymentService { public function pay(): void; }").unwrap();
        fs::write(dir.path().join("service.php"), "<?php namespace App\\Services; use App\\Contracts\\PaymentService as Contract; class StripeService implements Contract { public function pay(): void {} }").unwrap();
        let test = "<?php namespace App\\Test; use App\\Services\\StripeService; function testStripe(StripeService $service): void { $service->pay(); }";
        fs::write(dir.path().join("test.php"), test).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let offset = test.find("$service->pay").unwrap() + "$service->".len();
        let result = snapshot.definition_at_detailed(
            dir.path().join("test.php"),
            test,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(1),
            },
        );
        let DefinitionResult::Resolved(candidate) = result.result else {
            panic!("{result:?}")
        };
        assert!(candidate.location.file.ends_with("service.php"));
    }

    #[test]
    fn same_file_namespaced_typed_parameter_uses_imported_receiver() {
        let dir = tempfile::tempdir().unwrap();
        let test = r#"<?php
namespace App\One;
class StripeService { public function pay(): void {} }
namespace App\Two;
class StripeService { public function pay(): void {} }
namespace App\Test;
use App\Two\StripeService;
function test(StripeService $service): void { $service->pay(); }
"#;
        let path = dir.path().join("test.php");
        fs::write(&path, test).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let offset = test.rfind("$service->pay").unwrap() + "$service->".len();
        let result = snapshot.definition_at_detailed(
            &path,
            test,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(1),
            },
        );
        let DefinitionResult::Resolved(candidate) = result.result else {
            panic!("{result:?}")
        };
        assert!(candidate.location.file.ends_with("test.php"));
        assert!(candidate.location.span.start > test.find("App\\Two").unwrap());
    }

    #[test]
    fn member_resolver_walks_extends_chain_and_applies_overrides_visibility_and_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inheritance.php");
        let text = r#"<?php
namespace App;
class A {
    public function inherited() {}
    public function overridden() {}
    protected function guarded() {}
    private function hidden() {}
    public static function make() {}
    public string $value;
    public const KIND = 'a';
}
class B extends A {
    public function overridden() {}
    public function check(): void { $this->guarded(); $this->hidden(); }
}
class C extends B {}
function use_c(C $c): void { $c->inherited(); $c->overridden(); $c->make(); $c->value; C::KIND; }
class CycleA extends CycleB {}
class CycleB extends CycleA {}
"#;
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(30));
        let use_scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function && scope.class_name.is_none())
            .unwrap();
        let c = DeclaredType::Named {
            written: "C".into(),
            resolved: "App\\C".into(),
        };
        let resolver = snapshot.member_resolver();
        let inherited =
            resolver.resolve_method(use_scope.id, &c, "inherited", MemberAccess::Instance);
        let inherited_id = match inherited {
            MemberResolution::Resolved(id) => id,
            other => panic!("inherited method did not resolve: {other:?}"),
        };
        assert_eq!(
            snapshot.symbol(inherited_id).unwrap().fully_qualified_name,
            "App\\A::inherited"
        );
        let overridden =
            resolver.resolve_method(use_scope.id, &c, "overridden", MemberAccess::Instance);
        let overridden_id = match overridden {
            MemberResolution::Resolved(id) => id,
            other => panic!("override did not resolve: {other:?}"),
        };
        assert_eq!(
            snapshot.symbol(overridden_id).unwrap().fully_qualified_name,
            "App\\B::overridden"
        );
        assert!(matches!(
            resolver.resolve_method(use_scope.id, &c, "make", MemberAccess::Static),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            resolver.resolve_property(use_scope.id, &c, "value"),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            resolver.resolve_class_constant(use_scope.id, &c, "KIND"),
            MemberResolution::Resolved(_)
        ));

        let usage_offset = text.find("$c->inherited").unwrap() + "$c->".len();
        let semantic_definition = snapshot.definition_at(
            &path,
            text,
            usage_offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(30),
            },
        );
        let definition = match semantic_definition {
            DefinitionResult::Resolved(candidate) => candidate,
            other => panic!("semantic inherited definition did not resolve: {other:?}"),
        };
        assert_eq!(
            &fs::read_to_string(&path).unwrap()[definition.location.span],
            "inherited"
        );

        let check_scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| {
                scope.kind == ScopeKind::Method && scope.class_name.as_deref() == Some("App\\B")
            })
            .unwrap();
        assert!(matches!(
            resolver.resolve_binding_method(check_scope.id, "$this", "guarded"),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            resolver.resolve_binding_method(check_scope.id, "$this", "hidden"),
            MemberResolution::ResolvedButInaccessible(_)
        ));
        let parent_method = resolver.resolve_special_method(
            check_scope.id,
            "parent",
            "inherited",
            MemberAccess::Instance,
        );
        assert!(matches!(parent_method, MemberResolution::Resolved(_)));
        let self_override = resolver.resolve_special_method(
            check_scope.id,
            "self",
            "overridden",
            MemberAccess::Instance,
        );
        assert!(matches!(self_override, MemberResolution::Resolved(_)));
        let static_inherited =
            resolver.resolve_special_method(check_scope.id, "static", "make", MemberAccess::Static);
        assert!(matches!(static_inherited, MemberResolution::Resolved(_)));

        let cycle = DeclaredType::Named {
            written: "CycleA".into(),
            resolved: "App\\CycleA".into(),
        };
        assert!(matches!(
            resolver.resolve_method(use_scope.id, &cycle, "missing", MemberAccess::Instance),
            MemberResolution::Unresolved
        ));
    }

    #[test]
    fn definition_at_parent_class_scope_preserves_visibility_context() {
        let text = r#"<?php
class ParentBase {
    public function save(): void {}
    protected function guarded(): void {}
    private function secret(): void {}
}
class ParentChild extends ParentBase {
    public function execute(): void {
        parent::save();
        parent::guarded();
        parent::secret();
        $this->save();
        $this->guarded();
        $this->secret();
    }
}
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parent.php");
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(31));
        let context = |offset| {
            snapshot.definition_at_detailed(
                &path,
                text,
                offset,
                DefinitionQueryContext {
                    document_version: None,
                    semantic_revision: SemanticRevision(31),
                },
            )
        };
        for needle in [
            "parent::save",
            "parent::guarded",
            "$this->save",
            "$this->guarded",
        ] {
            let result = context(
                text.find(needle).unwrap()
                    + needle
                        .rfind("save")
                        .or_else(|| needle.rfind("guarded"))
                        .unwrap(),
            );
            assert!(
                matches!(result.result, DefinitionResult::Resolved(candidate) if candidate.confidence == DefinitionConfidence::Exact)
            );
            assert_eq!(result.outcome, SemanticDefinitionOutcome::Resolved);
        }
        for needle in ["parent::secret", "$this->secret"] {
            let result = context(text.find(needle).unwrap() + needle.len() - "secret".len());
            assert!(
                matches!(result.result, DefinitionResult::Resolved(candidate) if candidate.confidence == DefinitionConfidence::Partial)
            );
            assert_eq!(result.outcome, SemanticDefinitionOutcome::Inaccessible);
        }
    }

    #[test]
    fn member_resolver_inheritance_cross_file_and_namespace_import() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("Base.php");
        let child_path = dir.path().join("Child.php");
        fs::write(
            &base_path,
            "<?php namespace App\\Core; class Base { public function save(): void {} }",
        )
        .unwrap();
        let child_text = "<?php namespace App\\Service; use App\\Core\\Base; class Child extends Base {} function test(Child $child): void { $child->save(); }";
        fs::write(&child_path, child_text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(31));
        let offset = child_text.rfind("save()").unwrap();
        let result = snapshot.definition_at(
            &child_path,
            child_text,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(31),
            },
        );
        let candidate = match result {
            DefinitionResult::Resolved(candidate) => candidate,
            other => panic!("cross-file inherited definition did not resolve: {other:?}"),
        };
        assert_eq!(
            candidate.location.file,
            fs::canonicalize(&base_path).unwrap()
        );
        assert_eq!(
            &fs::read_to_string(&base_path).unwrap()[candidate.location.span],
            "save"
        );

        let global_dir = tempfile::tempdir().unwrap();
        let global_base = global_dir.path().join("Base.php");
        let global_child = global_dir.path().join("Child.php");
        fs::write(
            &global_base,
            "<?php class Base { public function save(): void {} }",
        )
        .unwrap();
        let global_text = "<?php class Child extends Base {} function test(Child $child): void { $child->save(); }";
        fs::write(&global_child, global_text).unwrap();
        let mut global_index = ProjectSymbolIndex::new();
        global_index.index_project(global_dir.path()).unwrap();
        let global_snapshot =
            SemanticSnapshot::from_project_index(&global_index, SemanticRevision(32));
        let global_result = global_snapshot.definition_at(
            &global_child,
            global_text,
            global_text.rfind("save()").unwrap(),
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(32),
            },
        );
        let global_candidate = match global_result {
            DefinitionResult::Resolved(candidate) => candidate,
            other => panic!("global cross-file inherited definition did not resolve: {other:?}"),
        };
        assert_eq!(
            global_candidate.location.file,
            fs::canonicalize(&global_base).unwrap()
        );
    }

    #[test]
    fn member_resolver_inheritance_three_levels_cross_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.php");
        let b = dir.path().join("B.php");
        let c = dir.path().join("C.php");
        let test = dir.path().join("test.php");
        fs::write(&a, "<?php class A { public function run(): void {} }").unwrap();
        fs::write(&b, "<?php class B extends A {}").unwrap();
        fs::write(&c, "<?php class C extends B {}").unwrap();
        let test_text = "<?php function test(C $c): void { $c->run(); }";
        fs::write(&test, test_text).unwrap();

        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(33));
        let a_id = snapshot
            .symbols_for_fqn("A")
            .iter()
            .copied()
            .find(|id| {
                snapshot
                    .symbol(*id)
                    .is_some_and(|s| s.kind == ProjectSymbolKind::Class)
            })
            .expect("A class must be indexed");
        let run = snapshot.members_named(a_id, "run", ProjectSymbolKind::Method);
        assert_eq!(run.len(), 1, "A::run must be in the owner member index");
        assert_eq!(
            snapshot.symbol(run[0]).unwrap().fully_qualified_name,
            "A::run"
        );

        let offset = test_text.rfind("run").unwrap();
        let result = snapshot.definition_at(
            &test,
            test_text,
            offset,
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(33),
            },
        );
        let candidate = match result {
            DefinitionResult::Resolved(candidate) => candidate,
            other => panic!("three-level cross-file definition did not resolve: {other:?}"),
        };
        assert_eq!(candidate.location.file, fs::canonicalize(a).unwrap());
    }

    #[test]
    fn member_resolver_inheritance_three_levels_namespace_cross_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("A.php"),
            "<?php namespace App\\Base; class A { public function run(): void {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("B.php"),
            "<?php namespace App\\Middle; use App\\Base\\A as ParentA; class B extends ParentA {}",
        )
        .unwrap();
        fs::write(
            dir.path().join("C.php"),
            "<?php namespace App\\Final; class C extends \\App\\Middle\\B {}",
        )
        .unwrap();
        let test_path = dir.path().join("test.php");
        let test_text = "<?php namespace App\\Test; use App\\Final\\C; function test(C $c): void { $c->run(); }";
        fs::write(&test_path, test_text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(34));
        assert_eq!(snapshot.symbols_for_fqn("App\\Base\\A").len(), 1);
        let result = snapshot.definition_at(
            &test_path,
            test_text,
            test_text.rfind("run").unwrap(),
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(34),
            },
        );
        let candidate = match result {
            DefinitionResult::Resolved(candidate) => candidate,
            other => panic!("namespace inheritance did not resolve: {other:?}"),
        };
        assert_eq!(
            candidate.location.file,
            fs::canonicalize(dir.path().join("A.php")).unwrap()
        );
    }

    #[test]
    fn interface_relations_cover_implementations_and_incremental_changes() {
        let dir = tempfile::tempdir().unwrap();
        let service = dir.path().join("Service.php");
        let base = dir.path().join("Base.php");
        let impl_a = dir.path().join("A.php");
        let impl_b = dir.path().join("B.php");
        let child = dir.path().join("Child.php");
        fs::write(
            &service,
            "<?php interface Service { public function run(): void; }",
        )
        .unwrap();
        fs::write(&base, "<?php class Base { public function run(): void {} }").unwrap();
        fs::write(&impl_a, "<?php class A extends Base implements Service {} class Leaf extends A {} class Override extends Base implements Service { public function run(): void {} }").unwrap();
        fs::write(
            &impl_b,
            "<?php class B implements Service { public function run(): void {} }",
        )
        .unwrap();
        fs::write(&child, "<?php interface ChildService extends Service {} class Child implements ChildService { public function run(): void {} }").unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(100));
        let service_id = snapshot.symbols_for_fqn("Service")[0];
        let method_id = snapshot.members_named(service_id, "run", ProjectSymbolKind::Method)[0];
        let direct_names: Vec<_> = snapshot
            .direct_implementers_of(service_id)
            .iter()
            .filter_map(|id| {
                snapshot
                    .symbol(*id)
                    .map(|s| s.fully_qualified_name.as_str())
            })
            .collect();
        assert_eq!(direct_names, vec!["A", "B", "Override"]);
        let implementer_names: Vec<_> = snapshot
            .implementers_of(service_id)
            .iter()
            .filter_map(|id| {
                snapshot
                    .symbol(*id)
                    .map(|s| s.fully_qualified_name.as_str())
            })
            .collect();
        assert_eq!(
            implementer_names,
            vec!["A", "B", "Child", "Leaf", "Override"]
        );
        let implementation_names: Vec<_> = snapshot
            .implementations_of(method_id)
            .iter()
            .filter_map(|id| {
                snapshot
                    .symbol(*id)
                    .map(|s| s.fully_qualified_name.as_str())
            })
            .collect();
        assert_eq!(
            implementation_names,
            vec!["B::run", "Base::run", "Child::run", "Override::run"]
        );

        let mut builder = SnapshotBuilder::from_snapshot(&snapshot);
        builder.replace_workspace_file(&impl_b, "<?php class B {}".to_owned());
        let changed = builder.finish();
        assert!(
            !changed
                .implementers_of(service_id)
                .iter()
                .filter_map(|id| changed.symbol(*id))
                .any(|symbol| symbol.fully_qualified_name == "B")
        );
        assert!(changed.implementations_of(method_id).iter().all(|id| {
            changed
                .symbol(*id)
                .is_some_and(|s| s.fully_qualified_name != "B::run")
        }));
    }

    #[test]
    fn interface_relations_resolve_namespace_alias_and_multiple_interfaces() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Contracts.php"), "<?php namespace App\\Contracts; interface Service { public function run(): void; } interface Extra {} ").unwrap();
        fs::write(dir.path().join("Impl.php"), "<?php namespace App\\Impl; use App\\Contracts\\Service as Contract; use App\\Contracts\\Extra; class Impl implements Contract, Extra { public function run(): void {} } ").unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(101));
        let service = snapshot.symbols_for_fqn("App\\Contracts\\Service")[0];
        let extra = snapshot.symbols_for_fqn("App\\Contracts\\Extra")[0];
        let impl_id = snapshot.symbols_for_fqn("App\\Impl\\Impl")[0];
        let method = snapshot.members_named(service, "run", ProjectSymbolKind::Method)[0];
        assert_eq!(snapshot.direct_implementers_of(service), &[impl_id]);
        assert_eq!(snapshot.implementers_of(service), &[impl_id]);
        assert_eq!(
            snapshot.implementations_of(method),
            &[snapshot.symbols_for_fqn("App\\Impl\\Impl::run")[0]]
        );
        assert_eq!(snapshot.implementers_of(extra), &[impl_id]);
    }

    #[test]
    fn definition_at_resolves_inherited_constants_and_static_calls_semantically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.php");
        let text = "<?php class A { public const TYPE = 1; public static function create() {} } class B extends A {} class C extends B {} $x = C::TYPE; C::create(); C::class;";
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(35));
        let context = |_: usize| DefinitionQueryContext {
            document_version: None,
            semantic_revision: SemanticRevision(35),
        };
        let constant = snapshot.definition_at(&path, text, text.rfind("TYPE").unwrap(), context(0));
        let method =
            snapshot.definition_at(&path, text, text.find("create();").unwrap(), context(0));
        assert!(
            matches!(constant, DefinitionResult::Resolved(candidate) if candidate.location.span == index.symbols().iter().find(|s| s.fully_qualified_name == "A::TYPE").unwrap().range)
        );
        assert!(
            matches!(method, DefinitionResult::Resolved(candidate) if candidate.location.span == index.symbols().iter().find(|s| s.fully_qualified_name == "A::create").unwrap().range)
        );
        assert!(matches!(
            snapshot.definition_at(&path, text, text.rfind("class").unwrap(), context(0)),
            DefinitionResult::Resolved(_)
        ));
    }

    #[test]
    fn incremental_inherited_member_add_remove_and_signature_update() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.php");
        let b = dir.path().join("B.php");
        let c = dir.path().join("C.php");
        let test = dir.path().join("test.php");
        let a_v1 = "<?php class A { public function run(): void {} }";
        let a_v2 = "<?php class A { public function run(): void {} public function run2(): void {} public string $name; public const TYPE = 'x'; }";
        let a_v3 = "<?php class A { public function run(): void {} public function run2(int $value): int { return $value; } }";
        fs::write(&a, a_v1).unwrap();
        fs::write(&b, "<?php class B extends A {}").unwrap();
        fs::write(&c, "<?php class C extends B {}").unwrap();
        let test_v1 = "<?php function test(C $c): void { $c->run(); }";
        fs::write(&test, test_v1).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(40));
        let c_scope = base
            .scopes
            .records
            .iter()
            .find(|scope| scope.class_name.as_deref() == Some("C"))
            .unwrap();
        let receiver = DeclaredType::Named {
            written: "C".into(),
            resolved: "C".into(),
        };
        assert!(matches!(
            base.member_resolver().resolve_method(
                c_scope.id,
                &receiver,
                "run2",
                MemberAccess::Instance
            ),
            MemberResolution::Unresolved
        ));

        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.replace_workspace_file(&a, a_v2);
        builder.replace_workspace_file(
            &test,
            "<?php function test(C $c): void { $c->run2(); $c->name; C::TYPE; }",
        );
        let next = builder.finish();
        let c_scope = next
            .scopes
            .records
            .iter()
            .find(|scope| scope.class_name.as_deref() == Some("C"))
            .unwrap();
        assert!(matches!(
            next.member_resolver().resolve_method(
                c_scope.id,
                &receiver,
                "run2",
                MemberAccess::Instance
            ),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            next.member_resolver()
                .resolve_property(c_scope.id, &receiver, "name"),
            MemberResolution::Resolved(_)
        ));
        assert!(matches!(
            next.member_resolver()
                .resolve_class_constant(c_scope.id, &receiver, "TYPE"),
            MemberResolution::Resolved(_)
        ));

        let mut changed = SnapshotBuilder::from_snapshot(&next);
        changed.replace_workspace_file(&a, a_v3);
        let changed = changed.finish();
        let run2 = changed
            .symbols_for_fqn("A::run2")
            .first()
            .and_then(|id| changed.symbol(*id))
            .unwrap();
        assert_eq!(run2.parameters.as_deref(), Some("(int $value)"));
        assert_eq!(run2.return_type.as_deref(), Some("int"));

        let mut removed = SnapshotBuilder::from_snapshot(&changed);
        removed.replace_workspace_file(&a, a_v1);
        let removed = removed.finish();
        let c_scope = removed
            .scopes
            .records
            .iter()
            .find(|scope| scope.class_name.as_deref() == Some("C"))
            .unwrap();
        assert!(matches!(
            removed.member_resolver().resolve_method(
                c_scope.id,
                &receiver,
                "run2",
                MemberAccess::Instance
            ),
            MemberResolution::Unresolved
        ));
    }

    #[test]
    fn expression_resolver_supports_new_and_return_type_call_chains() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Future.php"),
            "<?php namespace App; class Future { public function await(): mixed {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("Factory.php"),
            "<?php namespace App; class Factory { public function create(): Future {} } function execute(Factory $factory) {}",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        let resolver = ExpressionResolver::new(&snapshot, scope.id);
        let chain = Expression::MethodCall {
            receiver: Box::new(Expression::MethodCall {
                receiver: Box::new(Expression::Variable("$factory".into())),
                name: "create".into(),
                access: MemberAccess::Instance,
            }),
            name: "await".into(),
            access: MemberAccess::Instance,
        };
        assert_eq!(
            resolver.infer_expression_type(&chain),
            Some(DeclaredType::Builtin(BuiltinType::Mixed))
        );
        assert_eq!(
            resolver.infer_expression_type(&Expression::New("Factory".into())),
            Some(DeclaredType::Named {
                written: "Factory".into(),
                resolved: "App\\Factory".into()
            })
        );
    }

    #[test]
    fn simple_new_assignment_registers_local_receiver_type() {
        let text = "<?php namespace App; class Future { public function await() {} } function run() { $future = new Future(); return $future; }";
        let snapshot =
            SnapshotBuilder::from_php_text("assign.php", text, SemanticRevision(1)).build();
        let scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Function)
            .unwrap();
        assert_eq!(
            snapshot
                .lookup_binding(scope.id, "$future")
                .unwrap()
                .declared_type,
            Some(DeclaredType::Named {
                written: "Future".into(),
                resolved: "App\\Future".into()
            })
        );
    }

    #[test]
    fn global_new_assignment_registers_in_namespace_scope() {
        let text = "<?php namespace App; class Child {} $child = new Child();";
        let snapshot =
            SnapshotBuilder::from_php_text("global.php", text, SemanticRevision(1)).build();
        let namespace = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::Namespace && scope.namespace == "App")
            .unwrap();
        assert_eq!(
            snapshot.lookup_binding(namespace.id, "$child").unwrap().declared_type,
            Some(DeclaredType::Named {
                written: "Child".into(),
                resolved: "App\\Child".into()
            })
        );
    }

    #[test]
    fn incremental_assignment_scope_is_isolated_to_replaced_file() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.php");
        let path_b = dir.path().join("b.php");
        let text_a = format!(
            "<?php\nnamespace A;\nfunction overlap() {{\n{}\n}}\n",
            "    ".repeat(80)
        );
        let text_b_initial = "<?php namespace TraitTest;";
        let text_b = "<?php\nnamespace TraitTest;\n\nclass Base {}\n\nclass Child extends Base {}\n\n$child = new Child();\n$child->\n";

        let mut base_builder = SnapshotBuilder::empty(SemanticRevision(1));
        base_builder.replace_workspace_file(&path_a, &text_a);
        base_builder.replace_workspace_file(&path_b, text_b_initial);
        let base = base_builder.finish();

        let mut incremental = SnapshotBuilder::from_snapshot(&base);
        incremental.replace_workspace_file(&path_b, text_b);
        let snapshot = incremental.finish();

        let file_b = snapshot
            .files
            .records
            .iter()
            .find(|record| record.path == path_b)
            .map(|record| record.id)
            .expect("replaced file should remain indexed");
        let assignment_offset = text_b.find("$child =").unwrap();
        let completion_offset = text_b.find("$child->").unwrap();
        let scope_b = snapshot
            .scope_id_at_file(file_b, completion_offset)
            .expect("file B should have an applicable scope");

        assert!(
            text_a.len() > assignment_offset,
            "test fixture must overlap scope offsets across files"
        );
        let registrations: Vec<_> = snapshot
            .scopes
            .records
            .iter()
            .filter_map(|scope| {
                scope
                    .bindings
                    .iter()
                    .find(|binding| binding.name == "$child")
                    .map(|_binding| (scope.id, scope.file, scope.kind))
            })
            .collect();
        assert_eq!(
            snapshot.lookup_binding(scope_b, "$child").map(|binding| binding.name.as_str()),
            Some("$child"),
            "assignment must resolve in the replaced file's scope (scope_id={scope_b:?}, expected_file={file_b:?}, actual_scope_file={:?}, registrations={registrations:?})",
            snapshot.scopes.records[scope_b.0 as usize].file,
        );
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].1, Some(file_b));
    }

    #[test]
    fn global_new_assignment_without_namespace_uses_file_scope() {
        let text = "<?php class Child {} $child = new Child();";
        let snapshot =
            SnapshotBuilder::from_php_text("global.php", text, SemanticRevision(1)).build();
        let file = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| scope.kind == ScopeKind::File)
            .unwrap();
        assert!(snapshot.lookup_binding(file.id, "$child").is_some());
    }

    #[test]
    fn namespace_bindings_do_not_contaminate_each_other_and_local_shadows_global() {
        let text = r#"<?php
namespace One;
class A {}
$value = new A();
function run() { $value = new A(); return $value; }
namespace Two;
class B {}
$value = new B();
"#;
        let snapshot =
            SnapshotBuilder::from_php_text("namespaces.php", text, SemanticRevision(1)).build();
        let namespaces: Vec<_> = snapshot
            .scopes
            .records
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Namespace)
            .collect();
        let one = namespaces.iter().find(|scope| scope.namespace == "One").unwrap();
        let two = namespaces.iter().find(|scope| scope.namespace == "Two").unwrap();
        assert_eq!(
            snapshot.lookup_binding(one.id, "$value").unwrap().declared_type,
            Some(DeclaredType::Named { written: "A".into(), resolved: "One\\A".into() })
        );
        assert_eq!(
            snapshot.lookup_binding(two.id, "$value").unwrap().declared_type,
            Some(DeclaredType::Named { written: "B".into(), resolved: "Two\\B".into() })
        );
        let function = namespaces
            .iter()
            .flat_map(|scope| snapshot.scopes.records.iter().filter(move |child| {
                child.parent == Some(scope.id) && child.kind == ScopeKind::Function
            }))
            .next()
            .unwrap();
        assert_eq!(
            snapshot.lookup_binding(function.id, "$value").unwrap().declared_type,
            Some(DeclaredType::Named { written: "A".into(), resolved: "One\\A".into() })
        );
    }

    #[test]
    fn later_global_assignment_replaces_binding_in_same_scope() {
        let text = "<?php class A {} class B {} $value = new A(); $value = new B();";
        let snapshot =
            SnapshotBuilder::from_php_text("later.php", text, SemanticRevision(1)).build();
        let file = snapshot.scopes.records.iter().find(|scope| scope.kind == ScopeKind::File).unwrap();
        assert_eq!(
            snapshot.lookup_binding(file.id, "$value").unwrap().declared_type,
            Some(DeclaredType::Named { written: "B".into(), resolved: "B".into() })
        );
    }

    #[test]
    fn function_call_assignment_propagates_return_type_to_member_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("assign_call.php");
        fs::write(
            &path,
            "<?php namespace App; class Future { public function await() {} } function make(): Future { return new Future(); } function run() { $future = make(); return $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let method = snapshot.symbols_for_fqn("App\\Future::await")[0];
        assert_eq!(snapshot.references_for_target(method).len(), 1);
    }

    #[test]
    fn member_resolution_keeps_valid_duplicate_candidate_after_invalid_first_match() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("A.php"),
            "<?php namespace App; class Duplicate { private function run() {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("B.php"),
            "<?php namespace App; class Duplicate { public function run() {} }",
        )
        .unwrap();
        let caller = dir.path().join("Caller.php");
        fs::write(
            &caller,
            "<?php namespace App; function call(Duplicate $value) { return $value->run(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| {
                scope.kind == ScopeKind::Function
                    && scope.file == snapshot.file_id(&PersistentFileKey::workspace(&caller))
            })
            .unwrap();
        let result = snapshot
            .member_resolver()
            .resolve_binding_method(scope.id, "$value", "run");
        assert!(matches!(result, MemberResolution::Resolved(_)));
    }

    #[test]
    fn workspace_file_key_is_stable_before_and_after_unsaved_buffer_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("Buffer.php");
        let before = PersistentFileKey::workspace(&path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "<?php class Buffer {}\n").unwrap();
        let after = PersistentFileKey::workspace(&path);
        assert_eq!(before, after);
    }

    #[test]
    fn high_level_find_usages_bridge_accepts_declaration_and_usage_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.php");
        let text = "<?php namespace App; class Future { public function await() {} } function first(Future $future) { $future->await(); } function second(Future $future) { $future->await(); }";
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let engine = SemanticEngine::from_snapshot(snapshot);
        let declaration_offset = text.find("await()").unwrap();
        let usage_offset = text.rfind("$future->await").unwrap() + "$future->".len();
        let declaration_result =
            engine.find_usages_at(&path, declaration_offset, FindUsagesOptions::default());
        let usage_result = engine.find_usages_at(&path, usage_offset, FindUsagesOptions::default());
        assert_eq!(declaration_result.status, FindUsagesStatus::Complete);
        assert_eq!(usage_result.status, FindUsagesStatus::Complete);
        assert_eq!(declaration_result.usages.len(), 2);
        assert_eq!(usage_result.usages.len(), 2);
        assert!(
            declaration_result
                .usages
                .iter()
                .all(|usage| usage.provider == ReferenceProvider::Semantic)
        );
    }

    #[test]
    fn definition_at_resolves_future_type_and_method_from_byte_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let future_path = dir.path().join("Future.php");
        let client_path = dir.path().join("functions.php");
        fs::write(
            &future_path,
            "<?php namespace Omegaalfa\\FiberEventLoop; class Future { public function await(): mixed {} }",
        )
        .unwrap();
        let client = "<?php namespace Omegaalfa\\HttpClient\\Http; use Omegaalfa\\FiberEventLoop\\Future; function await(Future $future): mixed { return $future->await(); }";
        fs::write(&client_path, client).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(9));
        let context = DefinitionQueryContext {
            document_version: Some(1),
            semantic_revision: SemanticRevision(9),
        };
        let type_offset = client.find("Future $future").unwrap();
        let method_offset = client.rfind("await()").unwrap();
        let type_result = snapshot.definition_at(&client_path, client, type_offset, context);
        let method_result = snapshot.definition_at(&client_path, client, method_offset, context);
        let type_target = match type_result {
            DefinitionResult::Resolved(candidate) => candidate.location,
            other => panic!("unexpected type result: {other:?}"),
        };
        let method_target = match method_result {
            DefinitionResult::Resolved(candidate) => candidate.location,
            other => panic!("unexpected method result: {other:?}"),
        };
        assert_eq!(type_target.file, fs::canonicalize(&future_path).unwrap());
        assert_eq!(method_target.file, fs::canonicalize(&future_path).unwrap());
        assert_eq!(
            &fs::read_to_string(&future_path).unwrap()[method_target.span],
            "await"
        );
        assert_eq!(
            snapshot
                .definition_at_detailed(&client_path, client, method_offset, context)
                .outcome,
            SemanticDefinitionOutcome::Resolved
        );
    }

    #[test]
    fn definition_at_resolves_every_byte_of_await_without_selecting_return() {
        let dir = tempfile::tempdir().unwrap();
        let future_path = dir.path().join("Future.php");
        let client_path = dir.path().join("functions.php");
        fs::write(
            &future_path,
            "<?php namespace Omegaalfa\\FiberEventLoop; class Future { public function await(): mixed {} }",
        )
        .unwrap();
        let client = "<?php\nnamespace Omegaalfa\\HttpClient\\Http;\nuse Omegaalfa\\FiberEventLoop\\Future;\nfunction await(Future $future): mixed\n{\n    return $future->await();\n}\n";
        fs::write(&client_path, client).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(10));
        let context = DefinitionQueryContext {
            document_version: None,
            semantic_revision: SemanticRevision(10),
        };
        let await_start = client.rfind("await()").unwrap();
        for offset in await_start..await_start + "await".len() {
            let result = snapshot.definition_at(&client_path, client, offset, context);
            let DefinitionResult::Resolved(candidate) = result else {
                panic!("offset {offset} did not resolve: {result:?}");
            };
            assert_eq!(
                candidate.location.file,
                fs::canonicalize(&future_path).unwrap()
            );
            assert_eq!(
                &fs::read_to_string(&future_path).unwrap()[candidate.location.span],
                "await"
            );
        }
        for offset in [await_start.saturating_sub(1), await_start + "await".len()] {
            let result = snapshot.definition_at(&client_path, client, offset, context);
            if let DefinitionResult::Resolved(candidate) = result {
                assert_ne!(
                    candidate.location.span,
                    await_start..await_start + "await".len()
                );
            }
        }
    }

    #[test]
    fn definition_at_never_resolves_php_keywords_as_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keywords.php");
        let source = "<?php class Foo {} function f() { return new Foo; if (true) { foreach ([] as $x) {} } }";
        fs::write(&path, source).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(11));
        let context = DefinitionQueryContext {
            document_version: None,
            semantic_revision: SemanticRevision(11),
        };
        for keyword in ["return", "new", "function", "class", "if", "foreach"] {
            let offset = source.find(keyword).unwrap();
            assert!(
                matches!(
                    snapshot.definition_at(&path, source, offset, context),
                    DefinitionResult::Unresolved
                ),
                "keyword {keyword:?} must not become a definition"
            );
        }
    }

    #[test]
    fn definition_audit_covers_functions_constants_and_stale_buffers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("functions.php");
        let saved =
            "<?php namespace App; function foo(): void {} const VALUE = 1; foo(); echo VALUE;";
        fs::write(&path, saved).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(4));
        let context = DefinitionQueryContext {
            document_version: Some(1),
            semantic_revision: SemanticRevision(4),
        };
        let function_offset = saved.rfind("foo()").unwrap();
        let constant_offset = saved.rfind("VALUE").unwrap();
        let function_result =
            snapshot.definition_at_detailed(&path, saved, function_offset, context);
        assert!(matches!(
            function_result.result,
            DefinitionResult::Resolved(_)
        ));
        assert!(matches!(
            snapshot.definition_at(&path, saved, constant_offset, context),
            DefinitionResult::Resolved(_)
        ));

        let dirty =
            "<?php namespace App; function foo(): void {} const VALUE = 1; bar(); echo VALUE;";
        let dirty_result =
            snapshot.definition_at_detailed(&path, dirty, dirty.rfind("VALUE").unwrap(), context);
        assert_eq!(
            dirty_result.outcome,
            SemanticDefinitionOutcome::StaleSnapshot
        );
        assert!(matches!(dirty_result.result, DefinitionResult::Unresolved));

        let vendor_path = dir.path().join("vendor_usage.php");
        let vendor_text = "<?php namespace App; function send(\\Vendor\\Package\\Client $client): void { $client->send(); }";
        fs::write(&vendor_path, vendor_text).unwrap();
        let mut vendor_index = ProjectSymbolIndex::new();
        vendor_index.index_project(dir.path()).unwrap();
        let vendor_snapshot =
            SemanticSnapshot::from_project_index(&vendor_index, SemanticRevision(7));
        let vendor_result = vendor_snapshot.definition_at_detailed(
            &vendor_path,
            vendor_text,
            vendor_text.rfind("send()").unwrap(),
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(7),
            },
        );
        assert_eq!(
            vendor_result.outcome,
            SemanticDefinitionOutcome::DeferredVendor
        );
    }

    #[test]
    fn definition_audit_reports_ambiguity_and_incomplete_ast_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("A.php"),
            "<?php namespace App; class User {}",
        )
        .unwrap();
        fs::write(
            dir.path().join("B.php"),
            "<?php namespace App; class User {}",
        )
        .unwrap();
        let use_path = dir.path().join("Use.php");
        let use_text = "<?php namespace App; function use_it(): void { new User(); }";
        fs::write(&use_path, use_text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(5));
        let context = DefinitionQueryContext {
            document_version: None,
            semantic_revision: SemanticRevision(5),
        };
        let result = snapshot.definition_at_detailed(
            &use_path,
            use_text,
            use_text.find("User").unwrap(),
            context,
        );
        assert_eq!(result.outcome, SemanticDefinitionOutcome::Ambiguous);
        assert!(matches!(result.result, DefinitionResult::Candidates(_)));
        let user_ids = snapshot.symbols_for_fqn("App\\User");
        assert_eq!(snapshot.references.ambiguous_references.len(), 1);
        assert_eq!(
            snapshot
                .find_usages(user_ids[0], FindUsagesOptions::default())
                .status,
            FindUsagesStatus::Ambiguous
        );

        let incomplete_path = dir.path().join("Incomplete.php");
        let incomplete = "<?php namespace App; class User { public function run() {} } $service->";
        fs::write(&incomplete_path, incomplete).unwrap();
        let mut incomplete_index = ProjectSymbolIndex::new();
        incomplete_index.index_project(dir.path()).unwrap();
        let incomplete_snapshot =
            SemanticSnapshot::from_project_index(&incomplete_index, SemanticRevision(6));
        let incomplete_result = incomplete_snapshot.definition_at_detailed(
            &incomplete_path,
            incomplete,
            incomplete.len().saturating_sub(1),
            DefinitionQueryContext {
                document_version: None,
                semantic_revision: SemanticRevision(6),
            },
        );
        assert!(matches!(
            incomplete_result.outcome,
            SemanticDefinitionOutcome::IncompleteAst
                | SemanticDefinitionOutcome::Unresolved
                | SemanticDefinitionOutcome::MissingSymbol
        ));
    }

    #[test]
    fn reference_index_finds_future_method_calls_and_excludes_imports_by_default() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("Future.php"),
            "<?php namespace Omegaalfa\\FiberEventLoop; class Future { public function await(): mixed {} }",
        )
        .unwrap();
        let functions = r#"<?php
namespace Omegaalfa\HttpClient\Http;
use Omegaalfa\FiberEventLoop\Future;
function await(Future $future): mixed { return $future->await(); }
function second(Future $future): mixed { return $future->await(); }
"#;
        let functions_path = dir.path().join("functions.php");
        fs::write(&functions_path, functions).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(12));
        let method = snapshot
            .symbols_for_fqn("Omegaalfa\\FiberEventLoop\\Future::await")
            .first()
            .copied()
            .expect("Future::await must be indexed");
        let result = snapshot.find_usages(method, FindUsagesOptions::default());
        assert_eq!(result.status, FindUsagesStatus::Complete);
        assert_eq!(result.usages.len(), 2);
        assert!(
            result
                .usages
                .iter()
                .all(|usage| usage.role == ReferenceRole::MethodCall)
        );
        assert!(
            result
                .usages
                .iter()
                .all(|usage| usage.source_symbol.is_some())
        );
        assert!(result.usages.iter().all(|usage| usage.file.normalized_path
            == PersistentFileKey::workspace(&functions_path).normalized_path));
        let future_class = snapshot
            .symbols_for_fqn("Omegaalfa\\FiberEventLoop\\Future")
            .first()
            .copied()
            .unwrap();
        let class_usages = snapshot.find_usages(future_class, FindUsagesOptions::default());
        assert_eq!(
            class_usages
                .usages
                .iter()
                .filter(|usage| usage.role == ReferenceRole::ParameterType)
                .count(),
            2
        );
        assert!(
            class_usages
                .usages
                .iter()
                .all(|usage| usage.role != ReferenceRole::Import)
        );
        let class_with_import = snapshot.find_usages(
            future_class,
            FindUsagesOptions {
                include_imports: true,
                ..Default::default()
            },
        );
        assert!(
            class_with_import
                .usages
                .iter()
                .any(|usage| usage.role == ReferenceRole::Import)
        );
        let with_imports = snapshot.find_usages(
            method,
            FindUsagesOptions {
                include_imports: true,
                ..Default::default()
            },
        );
        assert_eq!(with_imports.usages.len(), 2);
    }

    #[test]
    fn reference_index_tracks_function_property_and_class_constant_roles() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"<?php
namespace App;
class User { public string $name; public const TYPE = 'user'; }
const GLOBAL = 1;
function foo(): void {}
function run(User $user): void { foo(); echo $user->name; $user->name = 'A'; echo User::TYPE; echo GLOBAL; }
"#;
        let path = dir.path().join("fixture.php");
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(13));
        let function = snapshot
            .symbols_for_fqn("App\\foo")
            .first()
            .copied()
            .unwrap();
        assert_eq!(
            snapshot
                .find_usages(function, FindUsagesOptions::default())
                .usages[0]
                .role,
            ReferenceRole::FunctionCall
        );
        let property = snapshot
            .symbols_for_fqn("App\\User::$name")
            .first()
            .copied()
            .unwrap();
        let property_roles: Vec<_> = snapshot
            .find_usages(property, FindUsagesOptions::default())
            .usages
            .into_iter()
            .map(|usage| usage.role)
            .collect();
        assert!(property_roles.contains(&ReferenceRole::PropertyRead));
        assert!(property_roles.contains(&ReferenceRole::PropertyWrite));
        let constant = snapshot
            .symbols_for_fqn("App\\User::TYPE")
            .first()
            .copied()
            .unwrap();
        assert_eq!(
            snapshot
                .find_usages(constant, FindUsagesOptions::default())
                .usages[0]
                .role,
            ReferenceRole::ClassConstantRead
        );
        let global = snapshot
            .symbols_for_fqn("App\\GLOBAL")
            .first()
            .copied()
            .unwrap();
        assert_eq!(
            snapshot
                .find_usages(global, FindUsagesOptions::default())
                .usages[0]
                .role,
            ReferenceRole::GlobalConstantRead
        );
    }

    #[test]
    fn reverse_reference_query_uses_target_index_for_large_fixture() {
        let mut builder = SnapshotBuilder::empty(SemanticRevision(14));
        let file_key = PersistentFileKey::workspace("synthetic.php");
        let file = FileId(0);
        builder.files.by_key.insert(file_key.clone(), file);
        builder.files.records.push(FileRecord {
            id: file,
            key: file_key.clone(),
            path: PathBuf::from("synthetic.php"),
            symbols: vec![SymbolId(0)],
        });
        let symbol_key = PersistentSymbolKey {
            file: file_key,
            kind: ProjectSymbolKind::Function,
            qualified_name: "App\\target".to_owned(),
            discriminator: None,
        };
        builder
            .symbols
            .by_key
            .insert(symbol_key.clone(), SymbolId(0));
        builder
            .symbols
            .by_fqn
            .insert("App\\target".to_owned(), vec![SymbolId(0)]);
        builder.symbols.records.push(SemanticSymbol {
            id: SymbolId(0),
            key: symbol_key,
            name: "target".to_owned(),
            fully_qualified_name: "App\\target".to_owned(),
            kind: ProjectSymbolKind::Function,
            file,
            range: 0..6,
            namespace: "App".to_owned(),
            visibility: Visibility::Unknown,
            modifiers: Vec::new(),
            parameters: None,
            return_type: None,
            owner: None,
            owner_key: None,
        });
        for offset in 0..100_000 {
            add_reference(
                &mut builder,
                file,
                offset..offset + 1,
                ScopeId(0),
                None,
                ReferenceRole::FunctionCall,
                ReferenceTarget::Resolved(SymbolId(0)),
            );
        }
        let snapshot = builder.build();
        let result = snapshot.find_usages(SymbolId(0), FindUsagesOptions::default());
        assert_eq!(result.status, FindUsagesStatus::Complete);
        assert_eq!(result.usages.len(), 100_000);
        assert_eq!(snapshot.references_for_target(SymbolId(0)).len(), 100_000);
    }

    #[test]
    fn incremental_reference_replacement_removes_stale_reverse_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.php");
        fs::write(
            &path,
            "<?php namespace App; class Future { public function await() {} } function first(Future $future) { return $future->await(); } function second(Future $future) { return $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(10));
        let method = base
            .symbols_for_fqn("App\\Future::await")
            .first()
            .copied()
            .unwrap();
        assert_eq!(base.references_for_target(method).len(), 2);
        let mut next = SnapshotBuilder::from_snapshot(&base);
        next.replace_workspace_file(
            &path,
            "<?php namespace App; class Future { public function await() {} } function first(Future $future) { return $future->await(); }",
        );
        let next = next.finish();
        assert_eq!(next.references_for_target(method).len(), 1);
        assert_eq!(base.references_for_target(method).len(), 2);
        assert!(next.revision > base.revision);
    }

    #[test]
    fn declarative_reference_roles_are_extracted_separately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.php");
        fs::write(
            &path,
            "<?php namespace App; use Framework\\BaseController; use App\\Contracts\\ServiceInterface; use App\\Traits\\Logging; use App\\Attributes\\Route; use Throwable; class Future { public function await() {} } class OtherFuture { public function await() {} } class Controller extends BaseController implements ServiceInterface { use Logging; public function execute(Future|OtherFuture $future): Result { try { if ($future instanceof Future) { return $future->await(); } } catch (Throwable $e) {} } } #[Route] class Attributed {}",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let roles: Vec<_> = snapshot
            .references
            .records
            .iter()
            .map(|reference| reference.role)
            .collect();
        for role in [
            ReferenceRole::Extends,
            ReferenceRole::Implements,
            ReferenceRole::TraitUse,
            ReferenceRole::Instanceof,
            ReferenceRole::CatchType,
            ReferenceRole::Attribute,
        ] {
            assert!(roles.contains(&role), "missing role {role:?}");
        }
        assert_eq!(
            roles
                .iter()
                .filter(|role| **role == ReferenceRole::ParameterType)
                .count(),
            2
        );
    }

    #[test]
    fn closures_and_arrow_functions_keep_type_and_member_references_in_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closures.php");
        fs::write(
            &path,
            "<?php namespace App; class Future { public function await() {} } class Result {} function make(): void { $fn = function (Future $future): Result { return $future->await(); }; $arrow = fn(Future $future): Result => $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let await_id = snapshot
            .symbols_for_fqn("App\\Future::await")
            .first()
            .copied()
            .unwrap();
        let await_refs: Vec<_> = snapshot
            .references_for_target(await_id)
            .iter()
            .filter_map(|id| snapshot.reference(*id))
            .collect();
        assert_eq!(await_refs.len(), 2);
        assert!(await_refs.iter().all(|reference| {
            reference.role == ReferenceRole::MethodCall
                && snapshot.scope(reference.source_scope).is_some_and(|scope| {
                    matches!(scope.kind, ScopeKind::Closure | ScopeKind::ArrowFunction)
                })
        }));
    }

    #[test]
    fn removing_a_file_clears_its_references_without_mutating_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.php");
        fs::write(
            &path,
            "<?php namespace App; class Future { public function await() {} } function use_it(Future $future) { return $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(2));
        let method = base.symbols_for_fqn("App\\Future::await")[0];
        assert_eq!(base.references_for_target(method).len(), 1);
        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.remove_file(&path);
        let next = builder.finish();
        assert!(next.references_for_target(method).is_empty());
        assert_eq!(base.references_for_target(method).len(), 1);
        assert!(next.symbols_for_fqn("App\\Future").is_empty());
        assert!(next.symbols_for_fqn("App\\Future::await").is_empty());
        assert!(next.file_id(&PersistentFileKey::workspace(&path)).is_none());
    }

    #[test]
    fn removing_a_file_clears_members_relations_and_external_targets() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("base.php");
        let impl_path = dir.path().join("impl.php");
        let usage_path = dir.path().join("usage.php");
        fs::write(
            &base_path,
            "<?php\nnamespace App;\ninterface Service { public function run(): void; }\nclass User {\n    public function save(): void {}\n    public string $name;\n    public const TYPE = 'x';\n}",
        )
        .unwrap();
        fs::write(
            &impl_path,
            "<?php namespace App; class Impl implements Service { public function run(): void {} }",
        )
        .unwrap();
        fs::write(
            &usage_path,
            "<?php namespace App; function test(User $user): void { $user->save(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(10));
        let user = base.symbols_for_fqn("App\\User")[0];
        let service = base.symbols_for_fqn("App\\Service")[0];
        let service_run = base.symbols_for_fqn("App\\Service::run")[0];
        assert_eq!(
            base.members_named(user, "save", ProjectSymbolKind::Method)
                .len(),
            1
        );
        assert_eq!(
            base.members_named(user, "name", ProjectSymbolKind::Property)
                .len(),
            1
        );
        assert_eq!(
            base.members_named(user, "TYPE", ProjectSymbolKind::Constant)
                .len(),
            1
        );
        assert_eq!(base.implementers_of(service).len(), 1);
        assert_eq!(base.implementations_of(service_run).len(), 1);

        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.remove_file(&impl_path);
        builder.remove_file(&base_path);
        let next = builder.finish();
        assert!(next.symbols_for_fqn("App\\User").is_empty());
        assert!(next.symbols_for_fqn("App\\Service").is_empty());
        assert!(
            next.members_named(user, "save", ProjectSymbolKind::Method)
                .is_empty()
        );
        assert!(
            next.members_named(user, "name", ProjectSymbolKind::Property)
                .is_empty()
        );
        assert!(
            next.members_named(user, "TYPE", ProjectSymbolKind::Constant)
                .is_empty()
        );
        assert!(next.implementers_of(service).is_empty());
        assert!(next.implementations_of(service_run).is_empty());
        let usage_file = next
            .file_id(&PersistentFileKey::workspace(&usage_path))
            .unwrap();
        for reference_id in next.references_for_file(usage_file) {
            let reference = next.reference(*reference_id).unwrap();
            assert!(!matches!(reference.target, ReferenceTarget::Resolved(id) if id == user));
        }
        assert!(!base.symbols_for_fqn("App\\User").is_empty());
        assert_eq!(base.implementers_of(service).len(), 1);
    }

    #[test]
    fn rename_batch_removes_old_path_and_replaces_new_path_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let old_path = dir.path().join("A.php");
        let new_path = dir.path().join("B.php");
        let text = "<?php namespace App; class User { public function save(): void {} }";
        fs::write(&old_path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(20));
        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.remove_file(&old_path);
        builder.replace_workspace_file(&new_path, text);
        let next = builder.finish();
        assert!(
            next.file_id(&PersistentFileKey::workspace(&old_path))
                .is_none()
        );
        let new_file = next
            .file_id(&PersistentFileKey::workspace(&new_path))
            .expect("renamed file should be active");
        assert_eq!(next.symbols_for_fqn("App\\User").len(), 1);
        let user = next.symbols_for_fqn("App\\User")[0];
        assert_eq!(next.symbol(user).unwrap().file, new_file);
        assert!(
            base.file_id(&PersistentFileKey::workspace(&old_path))
                .is_some()
        );
    }

    #[test]
    fn incremental_target_change_moves_reverse_reference_to_new_method() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.php");
        fs::write(
            &path,
            "<?php namespace App; class Future { public function await() {} } class OtherFuture { public function await() {} } function use_it(Future $future) { return $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(3));
        let future_method = base.symbols_for_fqn("App\\Future::await")[0];
        let other_method = base.symbols_for_fqn("App\\OtherFuture::await")[0];
        assert_eq!(base.references_for_target(future_method).len(), 1);
        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.replace_workspace_file(
            &path,
            "<?php namespace App; class Future { public function await() {} } class OtherFuture { public function await() {} } function use_it(OtherFuture $future) { return $future->await(); }",
        );
        let next = builder.finish();
        assert!(next.references_for_target(future_method).is_empty());
        assert_eq!(next.references_for_target(other_method).len(), 1);
    }

    #[test]
    fn incremental_batch_preserves_unrelated_file_references() {
        let dir = tempfile::tempdir().unwrap();
        let changed = dir.path().join("changed.php");
        let untouched = dir.path().join("untouched.php");
        fs::write(
            &changed,
            "<?php namespace App; class Future { public function await() {} } function first(Future $future) { return $future->await(); }",
        )
        .unwrap();
        fs::write(
            &untouched,
            "<?php namespace App; function second(Future $future) { return $future->await(); }",
        )
        .unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let base = SemanticSnapshot::from_project_index(&index, SemanticRevision(4));
        let method = base.symbols_for_fqn("App\\Future::await")[0];
        let untouched_file = base
            .file_id(&PersistentFileKey::workspace(&untouched))
            .unwrap();
        let before = base
            .references_for_file(untouched_file)
            .iter()
            .filter_map(|id| base.reference(*id))
            .map(|reference| (reference.span.clone(), reference.role))
            .collect::<Vec<_>>();
        let mut builder = SnapshotBuilder::from_snapshot(&base);
        builder.replace_workspace_file(
            &changed,
            "<?php namespace App; class Future { public function await() {} } function first(Future $future) { return $future->await(); } function extra() {}",
        );
        let next = builder.finish();
        let after_file = next
            .file_id(&PersistentFileKey::workspace(&untouched))
            .unwrap();
        let after = next
            .references_for_file(after_file)
            .iter()
            .filter_map(|id| next.reference(*id))
            .map(|reference| (reference.span.clone(), reference.role))
            .collect::<Vec<_>>();
        assert_eq!(before, after);
        assert_eq!(next.references_for_target(method).len(), 2);
    }

    #[test]
    fn completion_methods_for_binding_uses_resident_inheritance_and_traits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("completion.php");
        let text = r#"<?php
namespace App;
trait Shared {
    public function inheritedRun(): void {}
}
class OverrideBase {
    use Shared;
    public function baseOnly(): void {}
}
class OverrideChild extends OverrideBase {
    public function childOnly(): void {}
    public function inheritedRun(): void {}
}
function testOverride(OverrideChild $child): void {}
class CycleA extends CycleB {}
class CycleB extends CycleA { public function cycleMethod(): void {} }
function testCycle(CycleA $cycle): void {}
"#;
        fs::write(&path, text).unwrap();
        let mut index = ProjectSymbolIndex::new();
        index.index_project(dir.path()).unwrap();
        let snapshot = SemanticSnapshot::from_project_index(&index, SemanticRevision(1));
        let key = PersistentFileKey::workspace(&path);
        let offset = text.find("$child):").unwrap();
        let scope = snapshot.scope_id_at(&key, offset).expect("function scope");
        let methods =
            snapshot
                .member_resolver()
                .completion_methods_for_binding(scope, "$child", "inherited");
        let names = methods
            .iter()
            .filter_map(|id| snapshot.symbol(*id))
            .map(|symbol| symbol.fully_qualified_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["App\\OverrideChild::inheritedRun"]);

        let all = snapshot
            .member_resolver()
            .completion_methods_for_binding(scope, "$child", "");
        let all_names = all
            .iter()
            .filter_map(|id| snapshot.symbol(*id))
            .map(|symbol| symbol.name.as_str())
            .collect::<HashSet<_>>();
        assert!(all_names.contains("childOnly"));
        assert!(all_names.contains("baseOnly"));
        assert!(all_names.contains("inheritedRun"));
        assert_eq!(all_names.len(), all.len());

        let file_id = snapshot.file_id(&key).unwrap();
        assert_eq!(snapshot.scope_id_at_file(file_id, offset), Some(scope));

        let cycle_scope = snapshot
            .scopes
            .records
            .iter()
            .find(|scope| {
                scope.kind == ScopeKind::Function
                    && scope.bindings.iter().any(|b| b.name == "$cycle")
            })
            .expect("cycle function");
        let cycle_methods = snapshot.member_resolver().completion_methods_for_binding(
            cycle_scope.id,
            "$cycle",
            "cycle",
        );
        assert_eq!(cycle_methods.len(), 1);
    }
}
