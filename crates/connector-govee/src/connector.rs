use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ApplicableTarget, CapabilityStatus,
    ColumnDescriptor, ColumnValueType, ConnectionTestResult, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, ResourceItem, ResourceKindDescriptor,
    SetupGuide, SetupGuideVariant, SubTarget, WidgetBinding, WidgetLayout,
};
use serde_json::{json, Map, Value};

use crate::{
    config_schema, GoveeCapability, GoveeClient, GoveeConnectorConfig, GoveeDevice, GoveeError,
};

pub const TYPE_ID: &str = "govee";
pub const DISPLAY_NAME: &str = "Govee";
pub const ICON: &str = "lucide:lightbulb";

const API_HOST: &str = "openapi.api.govee.com";
const TARGET_PREFIX: &str = "device:";
const RESOURCE_ID_PARAM: &str = "resourceId";

const DATA_POINT_DEVICE_COUNT: &str = "deviceCount";
const DATA_POINT_POWER_STATE: &str = "powerState";
const DATA_POINT_BRIGHTNESS: &str = "brightness";
const DATA_POINT_COLOR_HEX: &str = "colorHex";
const DATA_POINT_COLOR_TEMPERATURE: &str = "colorTemperatureKelvin";

const ACTION_SET_POWER: &str = "setPower";
const ACTION_SET_BRIGHTNESS: &str = "setBrightness";
const ACTION_SET_COLOR: &str = "setColor";
const ACTION_SET_COLOR_TEMPERATURE: &str = "setColorTemperature";
const ACTION_APPLY_SCENE: &str = "apply";

const RESOURCE_KIND_DEVICES: &str = "devices";
const RESOURCE_KIND_SCENES: &str = "scenes";

const CAPABILITY_ON_OFF: &str = "devices.capabilities.on_off";
const CAPABILITY_RANGE: &str = "devices.capabilities.range";
const CAPABILITY_COLOR: &str = "devices.capabilities.color_setting";
const CAPABILITY_SCENE: &str = "devices.capabilities.dynamic_scene";
const INSTANCE_POWER: &str = "powerSwitch";
const INSTANCE_BRIGHTNESS: &str = "brightness";
const INSTANCE_COLOR_RGB: &str = "colorRgb";
const INSTANCE_COLOR_TEMPERATURE: &str = "colorTemperatureK";
const INSTANCE_SCENE: &str = "lightScene";

const TEST_CAPABILITY_LIST_DEVICES: &str = "list-devices";
const TEST_CAPABILITY_READ_DEVICE_STATE: &str = "read-device-state";
const TEST_CAPABILITY_SET_POWER: &str = "set-power";
const TEST_CAPABILITY_SET_BRIGHTNESS: &str = "set-brightness";
const TEST_CAPABILITY_SET_COLOR: &str = "set-color";
const TEST_CAPABILITY_SET_COLOR_TEMPERATURE: &str = "set-color-temperature";
const TEST_CAPABILITY_APPLY_SCENE: &str = "apply-scene";

/// Setup instructions published with the connector type catalog.
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        variants: vec![SetupGuideVariant {
            id: "api-key".to_owned(),
            label: "Connect your Govee account".to_owned(),
            description: "In the Govee Home app, open Settings, choose Apply for API Key, and submit the requested information. Govee now makes a newly generated key available immediately; generating another key invalidates the account's previous active key, so update every integration that used it. Enter the key in Loom's API key field. A Govee API key grants access across the account and cannot be narrowed with partial permissions."
                .to_owned(),
            // Account-key creation has no deployment snippet, so the shared
            // setup UI deliberately omits the empty template surface.
            template: String::new(),
            toggles: Vec::new(),
            capability_requirements: Vec::new(),
        }],
    }
}

/// One Govee account, with each device represented as a sub-target.
pub struct GoveeConnector {
    client: GoveeClient,
    devices: Arc<RwLock<Vec<GoveeDevice>>>,
}

impl GoveeConnector {
    /// Validates the key and verifies it by listing the account's devices.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = GoveeConnectorConfig::from_value(value)?;
        let client = GoveeClient::connect(&config.api_key).map_err(connector_error)?;
        let devices = client.devices().await.map_err(connector_error)?;
        Ok(Self {
            client,
            devices: Arc::new(RwLock::new(devices)),
        })
    }

    /// Builds a candidate for Test Connection without performing network I/O.
    pub fn from_config_value_for_connection_test(value: Value) -> Result<Self, ConnectorError> {
        let config = GoveeConnectorConfig::from_value(value)?;
        let client = GoveeClient::connect(&config.api_key).map_err(connector_error)?;
        Ok(Self {
            client,
            devices: Arc::new(RwLock::new(Vec::new())),
        })
    }

    #[cfg(test)]
    fn with_devices(client: GoveeClient, devices: Vec<GoveeDevice>) -> Self {
        Self {
            client,
            devices: Arc::new(RwLock::new(devices)),
        }
    }

    fn device_snapshot(&self) -> Vec<GoveeDevice> {
        self.devices
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn remember_devices(&self, devices: Vec<GoveeDevice>) {
        *self
            .devices
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = devices;
    }

    fn device_for_target(&self, target_id: &str) -> Option<GoveeDevice> {
        let id = device_id_from_target(target_id)?;
        self.device_snapshot()
            .into_iter()
            .find(|device| device.device == id)
    }

    async fn current_devices(&self) -> Result<Vec<GoveeDevice>, ConnectorError> {
        let devices = self.client.devices().await.map_err(connector_error)?;
        self.remember_devices(devices.clone());
        Ok(devices)
    }
}

#[async_trait]
impl loom_core::connector::Connector for GoveeConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        let devices = match self.current_devices().await {
            Ok(devices) => devices,
            Err(error) => {
                let mut details = Value::Object(Map::new());
                set_detail(&mut details, None, "error", json!(error.to_string()));
                return Ok(ConnectorStatus::new(HealthState::Down, details)
                    .with_target_health(String::new(), HealthState::Down));
            }
        };

        let mut details = Value::Object(Map::new());
        set_detail(
            &mut details,
            None,
            DATA_POINT_DEVICE_COUNT,
            json!(devices.len()),
        );
        let mut status = ConnectorStatus::new(HealthState::Healthy, details)
            .with_target_health(String::new(), HealthState::Healthy);
        let mut degraded = false;

        // Govee limits state reads per device, so polling sequentially is both
        // predictable and naturally gentle for typical home device counts.
        for device in &devices {
            let target_id = target_id(device);
            match self.client.state(device).await {
                Ok(states) => {
                    populate_device_details(&mut status.details, &target_id, device, &states);
                    status = status.with_target_health(target_id, HealthState::Healthy);
                }
                Err(error) => {
                    degraded = true;
                    set_detail(
                        &mut status.details,
                        Some(&target_id),
                        "error",
                        json!(error.to_string()),
                    );
                    status = status.with_target_health(target_id, HealthState::Down);
                }
            }
        }
        if degraded {
            status.health = HealthState::Degraded;
        }
        Ok(status)
    }

    async fn test_connection(&self) -> ConnectionTestResult {
        let devices = match self.client.devices().await {
            Ok(devices) => devices,
            Err(error) => {
                return ConnectionTestResult {
                    reachable: false,
                    capabilities: Vec::new(),
                    // GoveeClient keeps HTTP 401/403 authentication failures
                    // distinct from DNS, TLS, timeout and transport failures.
                    message: Some(error.to_string()),
                };
            }
        };
        self.remember_devices(devices.clone());

        let state_capability = match devices.first() {
            Some(device) => match self.client.state(device).await {
                Ok(_) => available_test_capability(
                    TEST_CAPABILITY_READ_DEVICE_STATE,
                    "Read device state",
                ),
                Err(error) => unavailable_test_capability(
                    TEST_CAPABILITY_READ_DEVICE_STATE,
                    "Read device state",
                    error.to_string(),
                ),
            },
            None => unavailable_test_capability(
                TEST_CAPABILITY_READ_DEVICE_STATE,
                "Read device state",
                "The account returned no devices to test.".to_owned(),
            ),
        };
        let state_available = state_capability.available;

        ConnectionTestResult {
            reachable: true,
            capabilities: vec![
                CapabilityStatus {
                    key: TEST_CAPABILITY_LIST_DEVICES.to_owned(),
                    label: "List devices".to_owned(),
                    available: true,
                    note: Some(format!("Found {} device(s).", devices.len())),
                },
                state_capability,
                available_test_capability(TEST_CAPABILITY_SET_POWER, "Set power"),
                available_test_capability(TEST_CAPABILITY_SET_BRIGHTNESS, "Set brightness"),
                available_test_capability(TEST_CAPABILITY_SET_COLOR, "Set colour"),
                available_test_capability(
                    TEST_CAPABILITY_SET_COLOR_TEMPERATURE,
                    "Set colour temperature",
                ),
                available_test_capability(TEST_CAPABILITY_APPLY_SCENE, "Apply scene"),
            ],
            message: Some(if state_available {
                format!(
                    "Authenticated successfully, found {} device(s), and verified a live device-state read. Available controls depend on each device's declared capabilities.",
                    devices.len()
                )
            } else {
                format!(
                    "Authenticated successfully and found {} device(s), but a live device-state read could not be verified.",
                    devices.len()
                )
            }),
        }
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        self.device_snapshot()
            .iter()
            .flat_map(actions_for_device)
            .collect()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        requested_target_id: Option<&str>,
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        let target_id =
            requested_target_id.ok_or_else(|| ConnectorError::invalid_action(action_id))?;
        let device = self
            .device_for_target(target_id)
            .ok_or_else(|| ConnectorError::invalid_action(action_id))?;

        let (capability_type, instance, value, message) = match action_id {
            ACTION_SET_POWER => {
                let capability =
                    require_capability(&device, CAPABILITY_ON_OFF, INSTANCE_POWER, action_id)?;
                let on = required_bool(action_id, &params, "on")?;
                (
                    CAPABILITY_ON_OFF.to_owned(),
                    INSTANCE_POWER.to_owned(),
                    enum_option_value(capability, if on { "on" } else { "off" })
                        .unwrap_or_else(|| json!(if on { 1 } else { 0 })),
                    format!(
                        "Set {} power {}",
                        device_label(&device),
                        if on { "on" } else { "off" }
                    ),
                )
            }
            ACTION_SET_BRIGHTNESS => {
                let capability =
                    require_capability(&device, CAPABILITY_RANGE, INSTANCE_BRIGHTNESS, action_id)?;
                let value = required_number(action_id, &params, "brightness")?;
                validate_range(action_id, "brightness", value, capability)?;
                (
                    CAPABILITY_RANGE.to_owned(),
                    INSTANCE_BRIGHTNESS.to_owned(),
                    json!(value),
                    format!("Set {} brightness", device_label(&device)),
                )
            }
            ACTION_SET_COLOR => {
                require_capability(&device, CAPABILITY_COLOR, INSTANCE_COLOR_RGB, action_id)?;
                let value = params
                    .get("colorHex")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid_param(action_id, "`colorHex` must be a #RRGGBB string")
                    })?;
                let packed =
                    hex_to_packed_rgb(value).map_err(|reason| invalid_param(action_id, &reason))?;
                (
                    CAPABILITY_COLOR.to_owned(),
                    INSTANCE_COLOR_RGB.to_owned(),
                    json!(packed),
                    format!("Set {} colour", device_label(&device)),
                )
            }
            ACTION_SET_COLOR_TEMPERATURE => {
                let capability = require_capability(
                    &device,
                    CAPABILITY_COLOR,
                    INSTANCE_COLOR_TEMPERATURE,
                    action_id,
                )?;
                let value = required_number(action_id, &params, "kelvin")?;
                validate_range(action_id, "kelvin", value, capability)?;
                (
                    CAPABILITY_COLOR.to_owned(),
                    INSTANCE_COLOR_TEMPERATURE.to_owned(),
                    json!(value),
                    format!("Set {} colour temperature", device_label(&device)),
                )
            }
            ACTION_APPLY_SCENE => {
                let resource_id = params
                    .get(RESOURCE_ID_PARAM)
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid_param(action_id, "`resourceId` must identify a scene")
                    })?;
                let scene = self
                    .client
                    .scenes(&device)
                    .await
                    .map_err(connector_error)?
                    .into_iter()
                    .find(|scene| scene_resource_id(scene) == resource_id)
                    .ok_or_else(|| {
                        invalid_param(action_id, "the selected scene no longer exists")
                    })?;
                (
                    scene.capability_type,
                    scene.instance,
                    scene.value,
                    format!("Applied scene {} to {}", scene.name, device_label(&device)),
                )
            }
            _ => return Err(ConnectorError::invalid_action(action_id)),
        };

        let request_id = self
            .client
            .control(&device, &capability_type, &instance, value)
            .await
            .map_err(connector_error)?;
        Ok(ActionResult::ok(message).with_payload(json!({ "requestId": request_id })))
    }

    fn supports_sub_targets(&self) -> bool {
        true
    }

    async fn list_sub_targets(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        Ok(self
            .current_devices()
            .await?
            .iter()
            .map(|device| {
                SubTarget::new(target_id(device), device_label(device))
                    .of_kind("light")
                    .with_icon(ICON)
            })
            .collect())
    }

    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        match target_id {
            None => vec![devices_kind()],
            Some(target)
                if self.device_for_target(target).is_some_and(|device| {
                    has_capability(&device, CAPABILITY_SCENE, INSTANCE_SCENE)
                }) =>
            {
                vec![scenes_kind()]
            }
            _ => Vec::new(),
        }
    }

    async fn list_resource_items(
        &self,
        kind: &str,
        requested_target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        match (kind, requested_target_id) {
            (RESOURCE_KIND_DEVICES, None) => Ok(self
                .device_snapshot()
                .iter()
                .map(|device| {
                    ResourceItem::new(target_id(device))
                        .with_field("name", device_label(device))
                        .with_field("model", device.sku.clone())
                })
                .collect()),
            (RESOURCE_KIND_SCENES, Some(target)) => {
                let Some(device) = self.device_for_target(target) else {
                    return Ok(Vec::new());
                };
                Ok(self
                    .client
                    .scenes(&device)
                    .await
                    .map_err(connector_error)?
                    .iter()
                    .map(|scene| {
                        ResourceItem::new(scene_resource_id(scene))
                            .with_field("name", scene.name.clone())
                    })
                    .collect())
            }
            _ => Ok(Vec::new()),
        }
    }

    fn config_schema(&self) -> Value {
        config_schema()
    }

    fn setup_guide(&self) -> Option<SetupGuide> {
        Some(setup_guide())
    }

    fn metadata(&self) -> ConnectorMetadata {
        ConnectorMetadata {
            id: TYPE_ID.to_owned(),
            name: DISPLAY_NAME.to_owned(),
            icon: Some(ICON.to_owned()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            min_size: (2, 2),
        }
    }

    fn display_fields(&self) -> Vec<DisplayField> {
        vec![DisplayField::new("API", API_HOST)]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        let mut points = vec![DataPointDescriptor::new(
            DATA_POINT_DEVICE_COUNT,
            "Devices",
            DataPointValueType::Number,
        )];
        for device in self.device_snapshot() {
            let target = target_id(&device);
            points.extend(
                data_points_for_device(&device)
                    .into_iter()
                    .map(|point| point.for_target(&target)),
            );
        }
        points
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![WidgetBinding::display(
            DATA_POINT_DEVICE_COUNT,
            DisplayWidgetType::StatTile,
        )])
    }

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        let Some(target_id) = target_id else {
            return self.default_layout();
        };
        let Some(device) = self.device_for_target(target_id) else {
            return WidgetLayout::default();
        };
        WidgetLayout::new(bindings_for_device(&device))
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        Some(NetworkTarget::new(API_HOST, 443))
    }
}

fn available_test_capability(key: &str, label: &str) -> CapabilityStatus {
    CapabilityStatus {
        key: key.to_owned(),
        label: label.to_owned(),
        available: true,
        note: Some("Available after account-wide API-key authentication.".to_owned()),
    }
}

fn unavailable_test_capability(key: &str, label: &str, note: String) -> CapabilityStatus {
    CapabilityStatus {
        key: key.to_owned(),
        label: label.to_owned(),
        available: false,
        note: Some(note),
    }
}

fn data_points_for_device(device: &GoveeDevice) -> Vec<DataPointDescriptor> {
    let mut points = Vec::new();
    if has_capability(device, CAPABILITY_ON_OFF, INSTANCE_POWER) {
        points.push(DataPointDescriptor::new(
            DATA_POINT_POWER_STATE,
            "Power",
            DataPointValueType::Bool,
        ));
    }
    if has_capability(device, CAPABILITY_RANGE, INSTANCE_BRIGHTNESS) {
        points.push(DataPointDescriptor::new(
            DATA_POINT_BRIGHTNESS,
            "Brightness",
            DataPointValueType::Number,
        ));
    }
    if has_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_RGB) {
        points.push(DataPointDescriptor::new(
            DATA_POINT_COLOR_HEX,
            "Colour",
            DataPointValueType::String,
        ));
    }
    if has_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_TEMPERATURE) {
        points.push(
            DataPointDescriptor::new(
                DATA_POINT_COLOR_TEMPERATURE,
                "Colour temperature",
                DataPointValueType::Number,
            )
            .with_unit("K"),
        );
    }
    points
}

fn actions_for_device(device: &GoveeDevice) -> Vec<ConnectorAction> {
    let target = target_id(device);
    let mut actions = Vec::new();
    if has_capability(device, CAPABILITY_ON_OFF, INSTANCE_POWER) {
        actions.push(action(
            ACTION_SET_POWER,
            "Set power",
            &target,
            json!({
                "type": "object", "properties": { "on": { "type": "boolean" } },
                "required": ["on"], "additionalProperties": false
            }),
        ));
    }
    if let Some(capability) = find_capability(device, CAPABILITY_RANGE, INSTANCE_BRIGHTNESS) {
        actions.push(action(
            ACTION_SET_BRIGHTNESS,
            "Set brightness",
            &target,
            numeric_schema("brightness", "Brightness", capability),
        ));
    }
    if has_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_RGB) {
        actions.push(action(
            ACTION_SET_COLOR,
            "Set colour",
            &target,
            json!({
                "type": "object", "properties": { "colorHex": {
                    "type": "string", "title": "Colour", "pattern": "^#[0-9A-Fa-f]{6}$"
                }}, "required": ["colorHex"], "additionalProperties": false
            }),
        ));
    }
    if let Some(capability) = find_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_TEMPERATURE)
    {
        actions.push(action(
            ACTION_SET_COLOR_TEMPERATURE,
            "Set colour temperature",
            &target,
            numeric_schema("kelvin", "Kelvin", capability),
        ));
    }
    actions
}

fn bindings_for_device(device: &GoveeDevice) -> Vec<WidgetBinding> {
    let mut bindings = Vec::new();
    if has_capability(device, CAPABILITY_ON_OFF, INSTANCE_POWER) {
        bindings.push(
            WidgetBinding::action(ACTION_SET_POWER, ActionWidgetType::Toggle).with_config(json!({
                "paramName": "on", "stateDataPointId": DATA_POINT_POWER_STATE
            })),
        );
    }
    if let Some(capability) = find_capability(device, CAPABILITY_RANGE, INSTANCE_BRIGHTNESS) {
        let (min, max, step) = capability_range(capability).unwrap_or((0.0, 100.0, 1.0));
        bindings.push(
            WidgetBinding::action(ACTION_SET_BRIGHTNESS, ActionWidgetType::Slider).with_config(
                json!({
                    "paramName": "brightness", "linkedDataPointId": DATA_POINT_BRIGHTNESS,
                    "min": min, "max": max, "step": step
                }),
            ),
        );
    }
    if has_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_RGB) {
        bindings.push(
            WidgetBinding::action(ACTION_SET_COLOR, ActionWidgetType::ColorPicker).with_config(
                json!({
                    "paramName": "colorHex", "linkedDataPointId": DATA_POINT_COLOR_HEX
                }),
            ),
        );
    }
    if let Some(capability) = find_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_TEMPERATURE)
    {
        let (min, max, step) = capability_range(capability).unwrap_or((2000.0, 9000.0, 1.0));
        bindings.push(
            WidgetBinding::action(ACTION_SET_COLOR_TEMPERATURE, ActionWidgetType::Slider)
                .with_config(json!({
                    "paramName": "kelvin", "linkedDataPointId": DATA_POINT_COLOR_TEMPERATURE,
                    "min": min, "max": max, "step": step
                })),
        );
    }
    bindings
}

fn devices_kind() -> ResourceKindDescriptor {
    // `user/devices` describes capabilities, not live state. A power column
    // would require one extra state request per row and therefore would not be
    // sourced from the already-fetched list as this host browser promises.
    ResourceKindDescriptor::new(
        RESOURCE_KIND_DEVICES,
        "Devices",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("model", "Model", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_rows_mapped_to_sub_targets()
}

fn scenes_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_SCENES,
        "Scenes",
        vec![ColumnDescriptor::new("name", "Name", ColumnValueType::Text)],
    )
    .applicable_to(ApplicableTarget::TargetOnly)
    .with_row_actions(vec![ConnectorAction {
        id: ACTION_APPLY_SCENE.to_owned(),
        target_id: None,
        label: "Apply".to_owned(),
        description: Some("Apply this Govee scene to the selected device.".to_owned()),
        params_schema: json!({
            "type": "object", "properties": {
                RESOURCE_ID_PARAM: { "type": "string", "minLength": 1 }
            }, "required": [RESOURCE_ID_PARAM], "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }])
}

fn populate_device_details(
    details: &mut Value,
    target_id: &str,
    device: &GoveeDevice,
    states: &[crate::client::GoveeCapabilityState],
) {
    for state in states {
        let data_point = match (state.kind.as_str(), state.instance.as_str()) {
            (CAPABILITY_ON_OFF, INSTANCE_POWER)
                if has_capability(device, CAPABILITY_ON_OFF, INSTANCE_POWER) =>
            {
                bool_state(&state.state.value).map(|value| (DATA_POINT_POWER_STATE, json!(value)))
            }
            (CAPABILITY_RANGE, INSTANCE_BRIGHTNESS)
                if has_capability(device, CAPABILITY_RANGE, INSTANCE_BRIGHTNESS) =>
            {
                state
                    .state
                    .value
                    .as_f64()
                    .map(|value| (DATA_POINT_BRIGHTNESS, json!(value)))
            }
            (CAPABILITY_COLOR, INSTANCE_COLOR_RGB)
                if has_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_RGB) =>
            {
                state
                    .state
                    .value
                    .as_u64()
                    .filter(|value| *value <= 0xFF_FFFF)
                    .map(|value| (DATA_POINT_COLOR_HEX, json!(packed_rgb_to_hex(value as u32))))
            }
            (CAPABILITY_COLOR, INSTANCE_COLOR_TEMPERATURE)
                if has_capability(device, CAPABILITY_COLOR, INSTANCE_COLOR_TEMPERATURE) =>
            {
                state
                    .state
                    .value
                    .as_f64()
                    .map(|value| (DATA_POINT_COLOR_TEMPERATURE, json!(value)))
            }
            _ => None,
        };
        if let Some((id, value)) = data_point {
            set_detail(details, Some(target_id), id, value);
        }
    }
}

fn action(id: &str, label: &str, target_id: &str, params_schema: Value) -> ConnectorAction {
    ConnectorAction {
        id: id.to_owned(),
        target_id: Some(target_id.to_owned()),
        label: label.to_owned(),
        description: None,
        params_schema,
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn numeric_schema(param: &str, title: &str, capability: &GoveeCapability) -> Value {
    let (min, max, step) = capability_range(capability).unwrap_or((0.0, 100.0, 1.0));
    json!({
        "type": "object", "properties": { param: {
            "type": "number", "title": title, "minimum": min, "maximum": max, "multipleOf": step
        }}, "required": [param], "additionalProperties": false
    })
}

fn find_capability<'a>(
    device: &'a GoveeDevice,
    kind: &str,
    instance: &str,
) -> Option<&'a GoveeCapability> {
    device
        .capabilities
        .iter()
        .find(|capability| capability.kind == kind && capability.instance == instance)
}

fn has_capability(device: &GoveeDevice, kind: &str, instance: &str) -> bool {
    find_capability(device, kind, instance).is_some()
}

fn require_capability<'a>(
    device: &'a GoveeDevice,
    kind: &str,
    instance: &str,
    action_id: &str,
) -> Result<&'a GoveeCapability, ConnectorError> {
    find_capability(device, kind, instance).ok_or_else(|| ConnectorError::invalid_action(action_id))
}

fn capability_range(capability: &GoveeCapability) -> Option<(f64, f64, f64)> {
    let range = capability.parameters.get("range")?;
    let min = range.get("min")?.as_f64()?;
    let max = range.get("max")?.as_f64()?;
    let precision = range
        .get("precision")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .unwrap_or(1.0);
    (min <= max).then_some((min, max, precision))
}

fn enum_option_value(capability: &GoveeCapability, name: &str) -> Option<Value> {
    capability
        .parameters
        .get("options")?
        .as_array()?
        .iter()
        .find(|option| option.get("name").and_then(Value::as_str) == Some(name))?
        .get("value")
        .cloned()
}

fn validate_range(
    action_id: &str,
    name: &str,
    value: f64,
    capability: &GoveeCapability,
) -> Result<(), ConnectorError> {
    let Some((min, max, step)) = capability_range(capability) else {
        return Err(invalid_param(
            action_id,
            "the device declared no usable range",
        ));
    };
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(invalid_param(
            action_id,
            &format!("`{name}` must be between {min} and {max}"),
        ));
    }
    let steps = (value - min) / step;
    if (steps - steps.round()).abs() > 1e-9 {
        return Err(invalid_param(
            action_id,
            &format!("`{name}` must use increments of {step} from {min}"),
        ));
    }
    Ok(())
}

fn required_bool(action_id: &str, params: &Value, name: &str) -> Result<bool, ConnectorError> {
    params
        .get(name)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid_param(action_id, &format!("`{name}` must be a boolean")))
}

fn required_number(action_id: &str, params: &Value, name: &str) -> Result<f64, ConnectorError> {
    params
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid_param(action_id, &format!("`{name}` must be a number")))
}

fn invalid_param(action_id: &str, reason: &str) -> ConnectorError {
    ConnectorError::InvalidParams {
        action_id: action_id.to_owned(),
        reason: reason.to_owned(),
    }
}

fn bool_state(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| match value.as_i64()? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    })
}

fn packed_rgb_to_hex(value: u32) -> String {
    format!("#{:06X}", value & 0xFF_FFFF)
}

fn hex_to_packed_rgb(value: &str) -> Result<u32, String> {
    let digits = value
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .ok_or_else(|| "`colorHex` must be a #RRGGBB string".to_owned())?;
    u32::from_str_radix(digits, 16).map_err(|_| "`colorHex` must be a #RRGGBB string".to_owned())
}

fn device_id_from_target(target_id: &str) -> Option<&str> {
    target_id
        .strip_prefix(TARGET_PREFIX)
        .filter(|id| !id.is_empty())
}

fn target_id(device: &GoveeDevice) -> String {
    format!("{TARGET_PREFIX}{}", device.device)
}

fn device_label(device: &GoveeDevice) -> String {
    let name = device.device_name.trim();
    if name.is_empty() {
        device.sku.clone()
    } else {
        name.to_owned()
    }
}

fn scene_resource_id(scene: &crate::client::GoveeScene) -> String {
    serde_json::to_string(&json!({
        "type": scene.capability_type,
        "instance": scene.instance,
        "value": scene.value
    }))
    .expect("scene resource identifiers contain only JSON values")
}

fn connector_error(error: GoveeError) -> ConnectorError {
    match error {
        GoveeError::ConnectionFailed(reason) => ConnectorError::unreachable(reason),
        GoveeError::AuthFailed(reason) => ConnectorError::AuthFailed { reason },
        GoveeError::RateLimited(reason) => ConnectorError::Internal(reason),
        GoveeError::ApiError { code, message } => {
            ConnectorError::Internal(format!("Govee API code {code}: {message}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use loom_core::connector::{Connector, WidgetBinding};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn device(capabilities: Vec<GoveeCapability>) -> GoveeDevice {
        GoveeDevice {
            sku: "H0000".to_owned(),
            device: "fixture-device".to_owned(),
            device_name: "Fixture light".to_owned(),
            device_type: "devices.types.light".to_owned(),
            capabilities,
        }
    }

    fn capability(kind: &str, instance: &str, parameters: Value) -> GoveeCapability {
        GoveeCapability {
            kind: kind.to_owned(),
            instance: instance.to_owned(),
            parameters,
        }
    }

    fn connector(device: GoveeDevice) -> GoveeConnector {
        GoveeConnector::with_devices(
            GoveeClient::connect("fixture-key").expect("client"),
            vec![device],
        )
    }

    async fn mock_router_api() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener");
        let address = listener.local_addr().expect("mock address");
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut request = vec![0_u8; 8192];
                let Ok(size) = stream.read(&mut request).await else {
                    continue;
                };
                let request = String::from_utf8_lossy(&request[..size]);
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let (field, data) = if path.ends_with("/user/devices") {
                    (
                        "data",
                        json!([{
                            "sku": "H0000", "device": "fixture-device",
                            "deviceName": "Fixture light", "type": "devices.types.light",
                            "capabilities": [
                                { "type": CAPABILITY_ON_OFF, "instance": INSTANCE_POWER, "parameters": {} },
                                { "type": CAPABILITY_RANGE, "instance": INSTANCE_BRIGHTNESS,
                                  "parameters": { "range": { "min": 1, "max": 100, "precision": 1 } } },
                                { "type": CAPABILITY_COLOR, "instance": INSTANCE_COLOR_RGB, "parameters": {} }
                            ]
                        }]),
                    )
                } else if path.ends_with("/device/state") {
                    (
                        "payload",
                        json!({ "capabilities": [
                            { "type": CAPABILITY_ON_OFF, "instance": INSTANCE_POWER, "state": { "value": 1 } },
                            { "type": CAPABILITY_RANGE, "instance": INSTANCE_BRIGHTNESS, "state": { "value": 50 } },
                            { "type": CAPABILITY_COLOR, "instance": INSTANCE_COLOR_RGB, "state": { "value": 16711680 } }
                        ]}),
                    )
                } else {
                    ("payload", json!({}))
                };
                let mut envelope = json!({ "code": 200, "msg": "success" });
                envelope
                    .as_object_mut()
                    .expect("fixture envelope")
                    .insert(field.to_owned(), data);
                let body = envelope.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn capabilities_gate_points_actions_and_composite_bindings() {
        let connector = connector(device(vec![
            capability(CAPABILITY_ON_OFF, INSTANCE_POWER, json!({})),
            capability(
                CAPABILITY_RANGE,
                INSTANCE_BRIGHTNESS,
                json!({ "range": { "min": 1, "max": 100, "precision": 1 } }),
            ),
            capability(CAPABILITY_COLOR, INSTANCE_COLOR_RGB, json!({})),
        ]));
        let target = "device:fixture-device";
        let point_ids = connector
            .data_points()
            .into_iter()
            .filter(|point| point.target_id.as_deref() == Some(target))
            .map(|point| point.id)
            .collect::<Vec<_>>();
        assert_eq!(
            point_ids,
            vec![
                DATA_POINT_POWER_STATE,
                DATA_POINT_BRIGHTNESS,
                DATA_POINT_COLOR_HEX
            ]
        );
        let action_ids = connector
            .actions()
            .await
            .into_iter()
            .map(|action| action.id)
            .collect::<Vec<_>>();
        assert_eq!(
            action_ids,
            vec![ACTION_SET_POWER, ACTION_SET_BRIGHTNESS, ACTION_SET_COLOR]
        );
        let layout = connector.default_layout_for(Some(target));
        assert_eq!(layout.bindings.len(), 3, "one layout is one composite tile");
        assert!(matches!(
            layout.bindings[0],
            WidgetBinding::Action {
                widget_type: ActionWidgetType::Toggle,
                ..
            }
        ));
        assert_eq!(
            layout.bindings[1],
            WidgetBinding::action(ACTION_SET_BRIGHTNESS, ActionWidgetType::Slider).with_config(
                json!({
                    "paramName": "brightness", "linkedDataPointId": DATA_POINT_BRIGHTNESS,
                    "min": 1.0, "max": 100.0, "step": 1.0
                })
            )
        );
    }

    #[tokio::test]
    async fn host_and_device_pass_the_shared_connector_contract() {
        let (base_url, server) = mock_router_api().await;
        let device = device(vec![
            capability(CAPABILITY_ON_OFF, INSTANCE_POWER, json!({})),
            capability(
                CAPABILITY_RANGE,
                INSTANCE_BRIGHTNESS,
                json!({ "range": { "min": 1, "max": 100, "precision": 1 } }),
            ),
            capability(CAPABILITY_COLOR, INSTANCE_COLOR_RGB, json!({})),
        ]);
        let connector = GoveeConnector::with_devices(
            GoveeClient::connect_at_unlimited("fixture-key", &base_url).expect("client"),
            vec![device],
        );
        loom_connector_test_kit::assert_connector_contract(
            &connector,
            &[None, Some("device:fixture-device".to_owned())],
        )
        .await;

        let test = connector.test_connection().await;
        assert!(test.reachable);
        let reported = test
            .capabilities
            .iter()
            .map(|capability| (capability.key.as_str(), capability.available))
            .collect::<std::collections::HashMap<_, _>>();
        for capability in [
            TEST_CAPABILITY_LIST_DEVICES,
            TEST_CAPABILITY_READ_DEVICE_STATE,
            TEST_CAPABILITY_SET_POWER,
            TEST_CAPABILITY_SET_BRIGHTNESS,
            TEST_CAPABILITY_SET_COLOR,
            TEST_CAPABILITY_SET_COLOR_TEMPERATURE,
            TEST_CAPABILITY_APPLY_SCENE,
        ] {
            assert_eq!(reported.get(capability), Some(&true), "{capability}");
        }
        server.abort();
    }

    #[test]
    fn setup_guide_uses_the_current_template_less_account_key_flow() {
        let guide = setup_guide();
        assert_eq!(guide.variants.len(), 1);
        let variant = &guide.variants[0];
        assert_eq!(variant.id, "api-key");
        assert_eq!(variant.label, "Connect your Govee account");
        assert!(variant.description.contains("Settings"));
        assert!(variant.description.contains("Apply for API Key"));
        assert!(variant.description.contains("available immediately"));
        assert!(variant.description.contains("account"));
        assert!(variant.template.is_empty());
        assert!(variant.toggles.is_empty());
        assert!(variant.capability_requirements.is_empty());
    }

    #[test]
    fn rgb_hex_and_packed_integer_round_trip_at_boundaries() {
        for value in [0, 1, 0x12_34_56, 0xFF_FFFF] {
            let hex = packed_rgb_to_hex(value);
            assert_eq!(hex_to_packed_rgb(&hex), Ok(value));
        }
        assert!(hex_to_packed_rgb("123456").is_err());
        assert!(hex_to_packed_rgb("#GG0000").is_err());
    }

    #[test]
    fn declared_device_ranges_are_enforced() {
        let whole_steps = capability(
            CAPABILITY_RANGE,
            INSTANCE_BRIGHTNESS,
            json!({ "range": { "min": 1, "max": 100, "precision": 1 } }),
        );
        assert!(validate_range(ACTION_SET_BRIGHTNESS, "brightness", 1.0, &whole_steps).is_ok());
        assert!(validate_range(ACTION_SET_BRIGHTNESS, "brightness", 101.0, &whole_steps).is_err());

        let half_steps = capability(
            CAPABILITY_RANGE,
            INSTANCE_BRIGHTNESS,
            json!({ "range": { "min": 1, "max": 10, "precision": 0.5 } }),
        );
        assert!(validate_range(ACTION_SET_BRIGHTNESS, "brightness", 2.5, &half_steps).is_ok());
        assert!(validate_range(ACTION_SET_BRIGHTNESS, "brightness", 2.25, &half_steps).is_err());
    }

    #[test]
    fn devices_rows_are_target_ids_and_scene_action_uses_resource_id() {
        let devices = devices_kind();
        assert!(devices.rows_map_to_sub_targets);
        assert_eq!(
            devices.columns.len(),
            2,
            "list response has no live power value"
        );
        let scenes = scenes_kind();
        assert_eq!(scenes.row_actions[0].id, ACTION_APPLY_SCENE);
        assert_eq!(
            scenes.row_actions[0].params_schema["required"],
            json!([RESOURCE_ID_PARAM])
        );
    }
}
