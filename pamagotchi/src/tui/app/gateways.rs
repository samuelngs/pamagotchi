use super::*;

impl App {
    pub async fn add_gateway(&mut self) {
        let Some(kind) = self
            .available_gateways
            .get(self.add_selection)
            .map(|gateway| gateway.kind.clone())
        else {
            self.show_add_dialog = false;
            self.add_selection = 0;
            return;
        };

        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::AddGateway {
                    request_id: request_id("add"),
                    kind,
                    vars: serde_json::Value::Object(Default::default()),
                })
                .await;
        }

        self.show_add_dialog = false;
        self.add_selection = 0;
    }

    pub fn selected_gateway(&self) -> Option<&GatewayView> {
        let id = self.selected_gateway_id.as_ref()?;
        self.gateways.iter().find(|gateway| &gateway.id == id)
    }

    pub fn selected_gateway_kind(&self) -> Option<&GatewayKindView> {
        let gateway = self.selected_gateway()?;
        self.available_gateways
            .iter()
            .find(|kind| kind.kind == gateway.kind)
    }

    pub fn selected_gateway_var_specs(&self) -> &[GatewayVarSpec] {
        self.selected_gateway_kind()
            .map(|kind| kind.vars.as_slice())
            .unwrap_or(&[])
    }

    pub fn selected_gateway_var_spec(&self) -> Option<&GatewayVarSpec> {
        self.selected_gateway_var_specs()
            .get(self.gateway_var_selection)
    }

    pub fn clamp_gateway_var_selection(&mut self) {
        let len = self.selected_gateway_var_specs().len();
        if len == 0 {
            self.gateway_var_selection = 0;
        } else if self.gateway_var_selection >= len {
            self.gateway_var_selection = len - 1;
        }
    }

    pub fn begin_gateway_var_edit(&mut self) {
        let Some(gateway) = self.selected_gateway() else {
            return;
        };
        let Some(spec) = self.selected_gateway_var_spec() else {
            return;
        };

        self.gateway_var_input = gateway_var_input_value(gateway, spec);
        self.gateway_var_cursor = self.gateway_var_input.len();
        self.editing_gateway_var = true;
    }

    pub fn cancel_gateway_var_edit(&mut self) {
        self.editing_gateway_var = false;
        self.gateway_var_input.clear();
        self.gateway_var_cursor = 0;
    }

    pub fn insert_gateway_var_char(&mut self, c: char) {
        self.gateway_var_cursor =
            clamp_to_char_boundary(&self.gateway_var_input, self.gateway_var_cursor);
        self.gateway_var_input.insert(self.gateway_var_cursor, c);
        self.gateway_var_cursor += c.len_utf8();
    }

    pub fn delete_gateway_var_char(&mut self) {
        self.gateway_var_cursor =
            clamp_to_char_boundary(&self.gateway_var_input, self.gateway_var_cursor);
        if self.gateway_var_cursor > 0 {
            let prev = self.gateway_var_input[..self.gateway_var_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.gateway_var_input.drain(prev..self.gateway_var_cursor);
            self.gateway_var_cursor = prev;
        }
    }

    pub fn move_gateway_var_cursor_left(&mut self) {
        self.gateway_var_cursor =
            clamp_to_char_boundary(&self.gateway_var_input, self.gateway_var_cursor);
        if self.gateway_var_cursor > 0 {
            self.gateway_var_cursor = self.gateway_var_input[..self.gateway_var_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_gateway_var_cursor_right(&mut self) {
        self.gateway_var_cursor =
            clamp_to_char_boundary(&self.gateway_var_input, self.gateway_var_cursor);
        if self.gateway_var_cursor < self.gateway_var_input.len() {
            self.gateway_var_cursor = self.gateway_var_input[self.gateway_var_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.gateway_var_cursor + i)
                .unwrap_or(self.gateway_var_input.len());
        }
    }

    pub async fn commit_gateway_var_edit(&mut self) {
        let Some(spec) = self.selected_gateway_var_spec().cloned() else {
            self.cancel_gateway_var_edit();
            return;
        };
        let value = gateway_var_value_from_input(&spec, &self.gateway_var_input);
        self.update_selected_gateway_var(&spec.key, value).await;
        self.cancel_gateway_var_edit();
    }

    pub async fn toggle_selected_gateway_bool_var(&mut self) {
        let Some(gateway) = self.selected_gateway() else {
            return;
        };
        let Some(spec) = self.selected_gateway_var_spec().cloned() else {
            return;
        };
        if spec.kind != GatewayVarKind::Bool {
            return;
        }
        let current = gateway
            .vars
            .get(&spec.key)
            .and_then(serde_json::Value::as_bool)
            .or_else(|| spec.default.as_ref().and_then(serde_json::Value::as_bool))
            .unwrap_or(false);
        self.update_selected_gateway_var(&spec.key, serde_json::Value::Bool(!current))
            .await;
    }

    async fn update_selected_gateway_var(&mut self, key: &str, value: serde_json::Value) {
        let Some(gateway) = self.selected_gateway().cloned() else {
            return;
        };
        let mut vars = gateway.vars.as_object().cloned().unwrap_or_default();
        vars.insert(key.to_string(), value);
        let vars = serde_json::Value::Object(vars);

        if let Some(local) = self.gateways.iter_mut().find(|gw| gw.id == gateway.id) {
            local.vars = vars.clone();
        }

        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::UpdateGatewayVars {
                    request_id: request_id("vars"),
                    id: gateway.id,
                    vars,
                })
                .await;
        }
    }

    pub async fn remove_selected_gateway(&mut self) {
        let Some(id) = self.selected_gateway_id.clone() else {
            return;
        };
        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::RemoveGateway {
                    request_id: request_id("remove"),
                    id,
                })
                .await;
        }
    }

    pub async fn restart_selected_gateway(&mut self) {
        let Some(id) = self.selected_gateway_id.clone() else {
            return;
        };
        if let Some(api) = &self.api {
            let _ = api
                .send(ClientRequest::RestartGateway {
                    request_id: request_id("restart"),
                    id,
                })
                .await;
        }
    }
}
