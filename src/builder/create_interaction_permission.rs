use std::collections::HashMap;

use serde_json::Value;

use crate::internal::prelude::Number;
use crate::model::interactions::ApplicationCommandPermissionType;
use crate::model::Permissions;
use crate::utils;

/// A builder for creating a new [`ApplicationCommandInteractionDataPermission`].
///
/// All fields are required
///
/// [`ApplicationCommandInteractionDataPermission`]: crate::model::interactions::ApplicationCommandPermissionData
/// [`kind`]: Self::kind
#[derive(Clone, Debug, Default)]
pub struct CreateInteractionPermissions(pub HashMap<&'static str, Value>);

impl CreateInteractionPermissions {
    /// Creates a permission for the Interaction
    pub fn create_permission<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut CreateInteractionPermission) -> &mut CreateInteractionPermission,
    {
        let mut data = CreateInteractionPermission::default();
        f(&mut data);

        self.add_permission(data);

        self
    }

    /// Adds a permission for the interaction
    pub fn add_permission(&mut self, permission: CreateInteractionPermission) -> &mut Self {
        let new_data = utils::hashmap_to_json_map(permission.0);
        let permissions = self.0.entry("permissions").or_insert_with(|| Value::Array(Vec::new()));

        let permissions_array = permissions.as_array_mut().expect("Must be an array");

        permissions_array.push(Value::Object(new_data));

        self
    }

    /// Sets all the permissions for the interaction
    pub fn set_permissions(&mut self, permissions: Vec<CreateInteractionPermission>) -> &mut Self {
        let new_permissions = permissions
            .into_iter()
            .map(|f| Value::Object(utils::hashmap_to_json_map(f.0)))
            .collect::<Vec<Value>>();

        self.0.insert("permissions", Value::Array(new_permissions));

        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct CreateInteractionPermission(pub HashMap<&'static str, Value>);

impl CreateInteractionPermission {
    /// Set the ApplicationCommandPermissionType for the InteractionPermission.
    pub fn kind(&mut self, kind: ApplicationCommandPermissionType) -> &mut Self {
        self.0.insert("type", Value::Number(serde_json::Number::from(kind as u8)));
        self
    }

    // Set the ApplicationCommandPermissionId for the InteractionPermission
    pub fn id(&mut self, id: u64) -> &mut Self {
        self.0.insert("id", Value::String(id.to_string()));
        self
    }

    // Set the permissions
    pub fn permission(&mut self, permission: bool) -> &mut Self {
        self.0.insert("permission", Value::Bool(permission));
        self
    }
}