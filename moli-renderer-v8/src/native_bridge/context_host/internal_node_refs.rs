use super::*;

const JS_MAX_SAFE_INTEGER_TOKEN: u64 = (1_u64 << 53) - 1;
const MAX_INTERNAL_REFERENCE_TOKEN_ATTEMPTS: usize = 16;

impl PageScriptHost {
    pub(crate) fn register_internal_node_reference(&mut self, handle: DomHandle) -> Option<u64> {
        self.dom_host().node(handle)?;

        for _ in 0..MAX_INTERNAL_REFERENCE_TOKEN_ATTEMPTS {
            let token = random_js_safe_token()?;
            if !self.internal_reference_token_is_available(token) {
                continue;
            }
            self.internal_node_references.insert(token, handle);
            return Some(token);
        }

        None
    }

    pub(crate) fn take_internal_node_reference(&mut self, token: u64) -> Option<DomHandle> {
        let handle = self.internal_node_references.remove(&token)?;
        self.dom_host().node(handle).is_some().then_some(handle)
    }

    pub(crate) fn discard_internal_node_reference(&mut self, token: u64) {
        self.internal_node_references.remove(&token);
    }

    pub(crate) fn register_internal_inspector_value_reference<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
    ) -> Option<u64> {
        for _ in 0..MAX_INTERNAL_REFERENCE_TOKEN_ATTEMPTS {
            let token = random_js_safe_token()?;
            if !self.internal_reference_token_is_available(token) {
                continue;
            }
            self.internal_inspector_value_references
                .insert(token, v8::Global::new(scope, value));
            return Some(token);
        }
        None
    }

    pub(crate) fn take_internal_inspector_value_reference<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        token: u64,
    ) -> Option<v8::Local<'s, v8::Value>> {
        self.internal_inspector_value_references
            .remove(&token)
            .map(|value| v8::Local::new(scope, value))
    }

    pub(crate) fn discard_internal_inspector_value_reference(&mut self, token: u64) {
        self.internal_inspector_value_references.remove(&token);
    }

    fn internal_reference_token_is_available(&self, token: u64) -> bool {
        token != 0
            && !self.internal_node_references.contains_key(&token)
            && !self
                .internal_inspector_value_references
                .contains_key(&token)
    }
}

fn random_js_safe_token() -> Option<u64> {
    let mut bytes = [0_u8; 8];
    moli_crypto::fill_secure_random(&mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes) & JS_MAX_SAFE_INTEGER_TOKEN)
}
