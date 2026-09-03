//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

mod allowed_object_relationships;
mod editor_project;
mod interactive_rendering_simple;
mod object_configuring;
mod object_defaults;
mod object_info;
mod object_rendering;
mod possible_events;
mod project_file;
mod smart_naming;

// New operation system and AI integration
pub mod ai;
pub mod object_properties;
pub mod operations;
pub mod pool_validation;

pub use editor_project::EditorProject;
pub use interactive_rendering_simple::InteractiveMaskRenderer;
pub use object_configuring::ConfigurableObject;
pub use object_defaults::default_object;
pub use object_info::ObjectInfo;
pub use object_rendering::RenderableObject;

// Operation system exports
pub use ai::snapshot::PoolSnapshot;
pub use object_properties::{
    get_properties, PropertyAccess, PropertyAccessError, PropertyDescriptor, PropertyError,
    PropertySemantic,
};
pub use operations::{
    AppliedTransaction, Operation, OperationExecutor, OperationHistory, OperationTransaction,
};
pub use pool_validation::{ValidationDiagnostic, ValidationSeverity};
