use super::*;

impl LiveStylesheet {
    pub(crate) fn insert_rule(
        &self,
        rule_text: &str,
        index: usize,
    ) -> Result<(), CssRuleInsertError> {
        self.insert_native_rule(rule_text, index).map(drop)
    }

    fn insert_native_rule(
        &self,
        rule_text: &str,
        index: usize,
    ) -> Result<CssRule, CssRuleInsertError> {
        let previous_rule_count = self.top_level_rule_count();
        let rule = self.parse_top_level_rule_for_insert(rule_text, index)?;
        let inserts_namespace = matches!(rule, CssRule::Namespace(_));
        self.ensure_owned_contents_for_mutation();
        if index < previous_rule_count {
            self.shift_rule_wrapper_paths_for_top_level_insert(index);
        }
        let contents = self.current_contents();
        {
            let mut guard = self.stylesheet.shared_lock.write();
            contents
                .rules
                .write_with(&mut guard)
                .0
                .insert(index, rule.clone());
        }
        if inserts_namespace {
            refresh_native_stylesheet_namespaces_after_cssom_mutation(&self.stylesheet);
        }
        self.reconcile_import_edges();
        #[cfg(test)]
        note_native_top_level_mutation_for_test();
        if let Some(mutation) =
            self.rule_mutation_for_rule(rule.clone(), &[], RuleChangeKind::Insertion)
        {
            self.note_exact_rule_mutations(vec![mutation]);
        } else {
            self.note_contents_mutation();
        }
        Ok(rule)
    }

    pub(crate) fn delete_rule(&self, index: usize) -> Result<(), CssRuleInsertError> {
        let removes_namespace = self.top_level_rule_is_namespace(index)?;
        let removed = self.rule_mutation_at_path(&[index], RuleChangeKind::Removal);
        self.ensure_owned_contents_for_mutation();
        let contents = self.current_contents();
        {
            let mut guard = self.stylesheet.shared_lock.write();
            contents
                .rules
                .write_with(&mut guard)
                .remove_rule(index)
                .map_err(css_rule_insert_error)?;
        }
        if removes_namespace {
            refresh_native_stylesheet_namespaces_after_cssom_mutation(&self.stylesheet);
        }
        self.reconcile_import_edges();
        self.shift_rule_wrapper_paths_for_top_level_delete(index);
        #[cfg(test)]
        note_native_top_level_mutation_for_test();
        if let Some(removed) = removed {
            self.note_exact_rule_mutations(vec![removed]);
        } else {
            self.note_contents_mutation();
        }
        Ok(())
    }

    pub(crate) fn replace_rule(
        &self,
        rule_text: &str,
        index: usize,
    ) -> Result<(), CssRuleInsertError> {
        let replaces_namespace = self.top_level_rule_is_namespace(index)?;
        let rule = self.parse_top_level_rule_for_insert(rule_text, index)?;
        let removed = self.rule_mutation_at_path(&[index], RuleChangeKind::Removal);
        let inserted = self.rule_mutation_for_rule(rule.clone(), &[], RuleChangeKind::Insertion);
        let inserts_namespace = matches!(rule, CssRule::Namespace(_));
        self.ensure_owned_contents_for_mutation();
        let contents = self.current_contents();
        {
            let mut guard = self.stylesheet.shared_lock.write();
            let rules = contents.rules.write_with(&mut guard);
            let Some(existing) = rules.0.get_mut(index) else {
                return Err(CssRuleInsertError::IndexSize);
            };
            *existing = rule;
        }
        if replaces_namespace || inserts_namespace {
            refresh_native_stylesheet_namespaces_after_cssom_mutation(&self.stylesheet);
        }
        self.reconcile_import_edges();
        self.replace_rule_wrapper_bindings_at_path(&[index]);
        #[cfg(test)]
        note_native_top_level_mutation_for_test();
        match (removed, inserted) {
            (Some(removed), Some(inserted)) => {
                self.note_exact_rule_mutations(vec![removed, inserted]);
            }
            _ => self.note_contents_mutation(),
        }
        Ok(())
    }

    pub(crate) fn insert_nested_rule(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
        containing_rule_type_bits: u32,
        parse_relative_rule_type: Option<CssRuleType>,
    ) -> Result<(), CssRuleInsertError> {
        self.insert_native_nested_rule(
            parent_path,
            rule_text,
            index,
            containing_rule_type_bits,
            parse_relative_rule_type,
        )
        .map(drop)
    }

    fn insert_native_nested_rule(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
        containing_rule_type_bits: u32,
        parse_relative_rule_type: Option<CssRuleType>,
    ) -> Result<CssRule, CssRuleInsertError> {
        let previous_rule_count = self
            .nested_rule_count(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        let rule = self.parse_nested_rule_for_insert(
            parent_path,
            rule_text,
            index,
            containing_rule_type_bits,
            parse_relative_rule_type,
            false,
        )?;
        self.ensure_owned_contents_for_mutation();
        let child_rules = self
            .mutable_child_rules_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        if index < previous_rule_count {
            self.shift_rule_wrapper_paths_for_insert(parent_path, index);
        }
        {
            let mut guard = self.stylesheet.shared_lock.write();
            child_rules
                .write_with(&mut guard)
                .0
                .insert(index, rule.clone());
        }
        #[cfg(test)]
        note_native_nested_mutation_for_test();
        if let Some(mutation) =
            self.rule_mutation_for_rule(rule.clone(), parent_path, RuleChangeKind::Insertion)
        {
            self.note_exact_rule_mutations(vec![mutation]);
        } else {
            self.note_contents_mutation();
        }
        Ok(rule)
    }

    pub(crate) fn delete_nested_rule(
        &self,
        parent_path: &[usize],
        index: usize,
    ) -> Result<(), CssRuleInsertError> {
        let rule_count = self
            .nested_rule_count(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        if index >= rule_count {
            return Err(CssRuleInsertError::IndexSize);
        }
        let mut removed_path = parent_path.to_vec();
        removed_path.push(index);
        let removed = self.rule_mutation_at_path(&removed_path, RuleChangeKind::Removal);
        self.ensure_owned_contents_for_mutation();
        let child_rules = self
            .mutable_child_rules_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        {
            let mut guard = self.stylesheet.shared_lock.write();
            child_rules
                .write_with(&mut guard)
                .remove_rule(index)
                .map_err(css_rule_insert_error)?;
        }
        self.shift_rule_wrapper_paths_for_delete(parent_path, index);
        #[cfg(test)]
        note_native_nested_mutation_for_test();
        if let Some(removed) = removed {
            self.note_exact_rule_mutations(vec![removed]);
        } else {
            self.note_contents_mutation();
        }
        Ok(())
    }

    pub(crate) fn replace_nested_rule(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
        containing_rule_type_bits: u32,
        parse_relative_rule_type: Option<CssRuleType>,
    ) -> Result<(), CssRuleInsertError> {
        let rule = self.parse_nested_rule_for_insert(
            parent_path,
            rule_text,
            index,
            containing_rule_type_bits,
            parse_relative_rule_type,
            true,
        )?;
        let mut replaced_path = parent_path.to_vec();
        replaced_path.push(index);
        let removed = self.rule_mutation_at_path(&replaced_path, RuleChangeKind::Removal);
        let inserted =
            self.rule_mutation_for_rule(rule.clone(), parent_path, RuleChangeKind::Insertion);
        self.ensure_owned_contents_for_mutation();
        let child_rules = self
            .mutable_child_rules_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        {
            let mut guard = self.stylesheet.shared_lock.write();
            let rules = child_rules.write_with(&mut guard);
            let Some(existing) = rules.0.get_mut(index) else {
                return Err(CssRuleInsertError::IndexSize);
            };
            *existing = rule;
        }
        self.replace_rule_wrapper_bindings_at_path(&replaced_path);
        #[cfg(test)]
        note_native_nested_mutation_for_test();
        match (removed, inserted) {
            (Some(removed), Some(inserted)) => {
                self.note_exact_rule_mutations(vec![removed, inserted]);
            }
            _ => self.note_contents_mutation(),
        }
        Ok(())
    }

    pub(crate) fn insert_keyframe_rule(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
    ) -> Result<(), CssRuleInsertError> {
        self.insert_native_keyframe_rule(parent_path, rule_text, index)
            .map(drop)
    }

    fn insert_native_keyframe_rule(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
    ) -> Result<ServoArc<Locked<Keyframe>>, CssRuleInsertError> {
        let count = self
            .keyframe_rule_count(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        if index > count {
            return Err(CssRuleInsertError::IndexSize);
        }
        let contents = self.current_contents();
        let rule = Keyframe::parse(rule_text, &contents, &self.stylesheet.shared_lock)
            .map_err(|_| CssRuleInsertError::Syntax)?;
        self.ensure_owned_contents_for_mutation();
        let keyframes_rule = self
            .keyframes_rule_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        if index < count {
            self.shift_rule_wrapper_paths_for_insert(parent_path, index);
        }
        {
            let mut guard = self.stylesheet.shared_lock.write();
            keyframes_rule
                .write_with(&mut guard)
                .keyframes
                .insert(index, rule.clone());
        }
        #[cfg(test)]
        note_native_keyframe_mutation_for_test();
        if let Some(mutation) = self.rule_mutation_at_path(parent_path, RuleChangeKind::Generic) {
            self.note_exact_rule_mutations(vec![mutation]);
        } else {
            self.note_contents_mutation();
        }
        Ok(rule)
    }

    pub(crate) fn delete_keyframe_rule(
        &self,
        parent_path: &[usize],
        index: usize,
    ) -> Result<(), CssRuleInsertError> {
        let count = self
            .keyframe_rule_count(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        if index >= count {
            return Err(CssRuleInsertError::IndexSize);
        }
        self.ensure_owned_contents_for_mutation();
        let keyframes_rule = self
            .keyframes_rule_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        {
            let mut guard = self.stylesheet.shared_lock.write();
            keyframes_rule
                .write_with(&mut guard)
                .keyframes
                .remove(index);
        }
        self.shift_rule_wrapper_paths_for_delete(parent_path, index);
        #[cfg(test)]
        note_native_keyframe_mutation_for_test();
        if let Some(mutation) = self.rule_mutation_at_path(parent_path, RuleChangeKind::Generic) {
            self.note_exact_rule_mutations(vec![mutation]);
        } else {
            self.note_contents_mutation();
        }
        Ok(())
    }

    pub(crate) fn replace_keyframe_rule(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
    ) -> Result<(), CssRuleInsertError> {
        let count = self
            .keyframe_rule_count(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        if index >= count {
            return Err(CssRuleInsertError::IndexSize);
        }
        let contents = self.current_contents();
        let rule = Keyframe::parse(rule_text, &contents, &self.stylesheet.shared_lock)
            .map_err(|_| CssRuleInsertError::Syntax)?;
        self.ensure_owned_contents_for_mutation();
        let keyframes_rule = self
            .keyframes_rule_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        {
            let mut guard = self.stylesheet.shared_lock.write();
            keyframes_rule.write_with(&mut guard).keyframes[index] = rule;
        }
        let mut replaced_path = parent_path.to_vec();
        replaced_path.push(index);
        self.replace_rule_wrapper_bindings_at_path(&replaced_path);
        #[cfg(test)]
        note_native_keyframe_mutation_for_test();
        if let Some(mutation) = self.rule_mutation_at_path(parent_path, RuleChangeKind::Generic) {
            self.note_exact_rule_mutations(vec![mutation]);
        } else {
            self.note_contents_mutation();
        }
        Ok(())
    }

    pub(crate) fn set_media_rule_media(
        &self,
        rule_path: &[usize],
        media_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Media(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let media = crate::style_engine::media_list::parse_media_query_list_with_context(
            media_text,
            &self.base_url,
            self.quirks_mode,
        );
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Media(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        *rule.media_queries.write_with(&mut guard) = media;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_import_rule_media(
        &self,
        rule_path: &[usize],
        media_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Import(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let media = crate::style_engine::media_list::parse_media_query_list_with_context(
            media_text,
            &self.base_url,
            self.quirks_mode,
        );
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Import(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let imported_stylesheet = {
            let guard = self.stylesheet.shared_lock.read();
            rule.read_with(&guard).stylesheet.as_sheet().cloned()
        }
        .ok_or(CssRuleInsertError::HierarchyRequest)?;
        let mut guard = imported_stylesheet.shared_lock.write();
        *imported_stylesheet.media.write_with(&mut guard) = media;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_font_feature_values_rule_font_family(
        &self,
        rule_path: &[usize],
        family_names: Vec<FamilyName>,
    ) -> Result<(), CssRuleInsertError> {
        self.mutate_font_feature_values_rule(rule_path, |rule| {
            rule.family_names = family_names;
        })
    }

    pub(crate) fn set_font_feature_values_rule_map_entry(
        &self,
        rule_path: &[usize],
        group: FontFeatureValuesMapGroup,
        name: &str,
        values: &[u32],
    ) -> Result<(), CssRuleInsertError> {
        let name = style::Atom::from(name);
        match group {
            FontFeatureValuesMapGroup::Annotation
            | FontFeatureValuesMapGroup::Ornaments
            | FontFeatureValuesMapGroup::Stylistic
            | FontFeatureValuesMapGroup::Swash => {
                let [value] = values else {
                    return Err(CssRuleInsertError::Syntax);
                };
                self.mutate_font_feature_values_rule(rule_path, |rule| {
                    let entries = match group {
                        FontFeatureValuesMapGroup::Annotation => &mut rule.annotation,
                        FontFeatureValuesMapGroup::Ornaments => &mut rule.ornaments,
                        FontFeatureValuesMapGroup::Stylistic => &mut rule.stylistic,
                        FontFeatureValuesMapGroup::Swash => &mut rule.swash,
                        _ => unreachable!("single-value group was checked above"),
                    };
                    update_font_feature_values_entry(entries, name, SingleValue(*value));
                })
            }
            FontFeatureValuesMapGroup::CharacterVariant => {
                let value = match values {
                    [first] => PairValues(*first, None),
                    [first, second] => PairValues(*first, Some(*second)),
                    _ => return Err(CssRuleInsertError::Syntax),
                };
                self.mutate_font_feature_values_rule(rule_path, |rule| {
                    update_font_feature_values_entry(&mut rule.character_variant, name, value);
                })
            }
            FontFeatureValuesMapGroup::Styleset => {
                if values.is_empty() {
                    return Err(CssRuleInsertError::Syntax);
                }
                let value = VectorValues(values.to_vec());
                self.mutate_font_feature_values_rule(rule_path, |rule| {
                    update_font_feature_values_entry(&mut rule.styleset, name, value);
                })
            }
        }
    }

    pub(crate) fn delete_font_feature_values_rule_map_entry(
        &self,
        rule_path: &[usize],
        group: FontFeatureValuesMapGroup,
        name: &str,
    ) -> Result<bool, CssRuleInsertError> {
        let name = style::Atom::from(name);
        let exists = self.font_feature_values_rule_at_path(rule_path, |rule| {
            font_feature_values_rule_has_entry(rule, group, &name)
        })?;
        if !exists {
            return Ok(false);
        }
        self.mutate_font_feature_values_rule(rule_path, |rule| match group {
            FontFeatureValuesMapGroup::Annotation => {
                delete_font_feature_values_entry(&mut rule.annotation, &name);
            }
            FontFeatureValuesMapGroup::Ornaments => {
                delete_font_feature_values_entry(&mut rule.ornaments, &name);
            }
            FontFeatureValuesMapGroup::Stylistic => {
                delete_font_feature_values_entry(&mut rule.stylistic, &name);
            }
            FontFeatureValuesMapGroup::Styleset => {
                delete_font_feature_values_entry(&mut rule.styleset, &name);
            }
            FontFeatureValuesMapGroup::CharacterVariant => {
                delete_font_feature_values_entry(&mut rule.character_variant, &name);
            }
            FontFeatureValuesMapGroup::Swash => {
                delete_font_feature_values_entry(&mut rule.swash, &name);
            }
        })?;
        Ok(true)
    }

    pub(crate) fn clear_font_feature_values_rule_map(
        &self,
        rule_path: &[usize],
        group: FontFeatureValuesMapGroup,
    ) -> Result<bool, CssRuleInsertError> {
        let is_empty = self.font_feature_values_rule_at_path(rule_path, |rule| match group {
            FontFeatureValuesMapGroup::Annotation => rule.annotation.is_empty(),
            FontFeatureValuesMapGroup::Ornaments => rule.ornaments.is_empty(),
            FontFeatureValuesMapGroup::Stylistic => rule.stylistic.is_empty(),
            FontFeatureValuesMapGroup::Styleset => rule.styleset.is_empty(),
            FontFeatureValuesMapGroup::CharacterVariant => rule.character_variant.is_empty(),
            FontFeatureValuesMapGroup::Swash => rule.swash.is_empty(),
        })?;
        if is_empty {
            return Ok(false);
        }
        self.mutate_font_feature_values_rule(rule_path, |rule| match group {
            FontFeatureValuesMapGroup::Annotation => rule.annotation.clear(),
            FontFeatureValuesMapGroup::Ornaments => rule.ornaments.clear(),
            FontFeatureValuesMapGroup::Stylistic => rule.stylistic.clear(),
            FontFeatureValuesMapGroup::Styleset => rule.styleset.clear(),
            FontFeatureValuesMapGroup::CharacterVariant => rule.character_variant.clear(),
            FontFeatureValuesMapGroup::Swash => rule.swash.clear(),
        })?;
        Ok(true)
    }

    pub(super) fn font_feature_values_rule_at_path<R>(
        &self,
        rule_path: &[usize],
        read: impl FnOnce(&FontFeatureValuesRule) -> R,
    ) -> Result<R, CssRuleInsertError> {
        top_level_rule_index(rule_path)?;
        let Some(NativeStylesheetRule::Css(CssRule::FontFeatureValues(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        Ok(read(rule.as_ref()))
    }

    fn mutate_font_feature_values_rule<R>(
        &self,
        rule_path: &[usize],
        mutate: impl FnOnce(&mut FontFeatureValuesRule) -> R,
    ) -> Result<R, CssRuleInsertError> {
        let index = top_level_rule_index(rule_path)?;
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::FontFeatureValues(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        self.ensure_owned_contents_for_mutation();
        let contents = self.current_contents();
        let result = {
            let mut guard = self.stylesheet.shared_lock.write();
            let rules = contents.rules.write_with(&mut guard);
            let Some(CssRule::FontFeatureValues(rule)) = rules.0.get_mut(index) else {
                return Err(CssRuleInsertError::HierarchyRequest);
            };
            mutate(ServoArc::make_mut(rule))
        };
        // FontFeatureValuesRule is held directly in a ServoArc rather than a
        // Locked node. make_mut() can replace that Arc when a V8 lease retains
        // the prior value, so refresh only the wrapper bound to this root.
        self.refresh_rule_wrapper_bindings_at_path(rule_path);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(result)
    }

    pub(crate) fn set_style_rule_declarations(
        &self,
        rule_path: &[usize],
        declaration_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Style(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let declarations = self.parse_declaration_block(declaration_text, CssRuleType::Style);
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Style(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let block = {
            let guard = self.stylesheet.shared_lock.read();
            rule.read_with(&guard).block.clone()
        };
        let mut guard = self.stylesheet.shared_lock.write();
        *block.write_with(&mut guard) = declarations;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::StyleRuleDeclarations);
        Ok(())
    }

    pub(crate) fn set_nested_declarations_rule_declarations(
        &self,
        rule_path: &[usize],
        declaration_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::NestedDeclarations(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let declarations = self.parse_declaration_block(declaration_text, CssRuleType::Style);
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::NestedDeclarations(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let block = {
            let guard = self.stylesheet.shared_lock.read();
            rule.read_with(&guard).block.clone()
        };
        let mut guard = self.stylesheet.shared_lock.write();
        *block.write_with(&mut guard) = declarations;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_keyframe_rule_declarations(
        &self,
        parent_path: &[usize],
        index: usize,
        declaration_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        let rule_path = child_rule_path(parent_path, index);
        if !matches!(
            self.native_rule_at_path(&rule_path),
            Some(NativeStylesheetRule::Keyframe(_))
        ) {
            return Err(CssRuleInsertError::IndexSize);
        }
        let declarations = self.parse_declaration_block(declaration_text, CssRuleType::Keyframe);
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Keyframe(rule)) = self.native_rule_at_path(&rule_path)
        else {
            return Err(CssRuleInsertError::IndexSize);
        };
        let block = {
            let guard = self.stylesheet.shared_lock.read();
            rule.read_with(&guard).block.clone()
        };
        let mut guard = self.stylesheet.shared_lock.write();
        *block.write_with(&mut guard) = declarations;
        drop(guard);
        self.note_rule_value_mutation_at_path(parent_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_font_face_rule_descriptors(
        &self,
        rule_path: &[usize],
        descriptor_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::FontFace(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let parsed = self.parse_font_face_descriptor_block(descriptor_text)?;
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::FontFace(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        let rule = rule.write_with(&mut guard);
        rule.descriptors = parsed.descriptors;
        rule.descriptor_importance = parsed.descriptor_importance;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_page_rule_descriptors(
        &self,
        rule_path: &[usize],
        descriptor_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Page(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let declarations = self.parse_declaration_block(descriptor_text, CssRuleType::Page);
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Page(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let block = {
            let guard = self.stylesheet.shared_lock.read();
            rule.read_with(&guard).block.clone()
        };
        let mut guard = self.stylesheet.shared_lock.write();
        *block.write_with(&mut guard) = declarations;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_page_rule_selectors(
        &self,
        rule_path: &[usize],
        selector_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Page(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let selectors = self.with_parser_context(CssRuleType::Page, None, |context| {
            let mut input = ParserInput::new(selector_text);
            let mut input = Parser::new(&mut input);
            input.parse_entirely(|input| PageSelectors::parse(context, input))
        });
        let selectors = selectors.map_err(|_| CssRuleInsertError::Syntax)?;
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Page(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        rule.write_with(&mut guard).selectors = selectors;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_page_margin_rule_descriptors(
        &self,
        rule_path: &[usize],
        descriptor_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        let rule_name = match self.native_rule_at_path(rule_path) {
            Some(NativeStylesheetRule::Css(CssRule::Margin(rule))) => {
                format!("@{}", rule.name())
            }
            _ => return Err(CssRuleInsertError::HierarchyRequest),
        };
        let declarations = self.parse_page_margin_descriptor_block(&rule_name, descriptor_text)?;
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Margin(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        *rule.block.write_with(&mut guard) = declarations;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_style_rule_selector(
        &self,
        rule_path: &[usize],
        selector_text: &str,
        containing_rule_type_bits: u32,
        parse_relative_rule_type: Option<CssRuleType>,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Style(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        let selectors = self.parse_style_rule_selectors(
            selector_text,
            containing_rule_type_bits,
            parse_relative_rule_type,
        )?;
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Style(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        rule.write_with(&mut guard).selectors = selectors;
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_keyframe_rule_selector(
        &self,
        parent_path: &[usize],
        index: usize,
        selector_text: &str,
    ) -> Result<(), CssRuleInsertError> {
        let rule_path = child_rule_path(parent_path, index);
        if !matches!(
            self.native_rule_at_path(&rule_path),
            Some(NativeStylesheetRule::Keyframe(_))
        ) {
            return Err(CssRuleInsertError::IndexSize);
        }
        let selector = parse_keyframe_selectors(selector_text)?;
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Keyframe(rule)) = self.native_rule_at_path(&rule_path)
        else {
            return Err(CssRuleInsertError::IndexSize);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        rule.write_with(&mut guard).selector = selector;
        drop(guard);
        self.note_rule_value_mutation_at_path(parent_path, RuleChangeKind::Generic);
        Ok(())
    }

    pub(crate) fn set_keyframes_rule_name(
        &self,
        rule_path: &[usize],
        name: &str,
    ) -> Result<(), CssRuleInsertError> {
        if !matches!(
            self.native_rule_at_path(rule_path),
            Some(NativeStylesheetRule::Css(CssRule::Keyframes(_)))
        ) {
            return Err(CssRuleInsertError::HierarchyRequest);
        }
        self.ensure_owned_contents_for_mutation();
        let Some(NativeStylesheetRule::Css(CssRule::Keyframes(rule))) =
            self.native_rule_at_path(rule_path)
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let mut guard = self.stylesheet.shared_lock.write();
        rule.write_with(&mut guard).name = KeyframesName::from_ident(name);
        drop(guard);
        self.note_rule_value_mutation_at_path(rule_path, RuleChangeKind::Generic);
        Ok(())
    }

    fn note_rule_value_mutation_at_path(&self, rule_path: &[usize], change_kind: RuleChangeKind) {
        #[cfg(test)]
        note_native_rule_value_mutation_for_test();
        if let Some(mutation) = self.rule_mutation_at_path(rule_path, change_kind) {
            self.note_exact_rule_mutations(vec![mutation]);
        } else {
            self.note_contents_mutation();
        }
    }

    fn note_exact_rule_mutations(&self, mutations: Vec<LiveStylesheetRuleMutation>) {
        debug_assert!(!mutations.is_empty());
        self.note_contents_mutation();
        let generation = self.cascade_generation();
        let mut batches = self.cascade_mutations.lock();
        let batch = batches
            .back_mut()
            .expect("a cascade mutation must publish one journal batch");
        debug_assert_eq!(batch.generation, generation);
        batch.mutation = LiveStylesheetCascadeMutation::Rules(mutations);
    }

    fn rule_mutation_at_path(
        &self,
        rule_path: &[usize],
        change_kind: RuleChangeKind,
    ) -> Option<LiveStylesheetRuleMutation> {
        let NativeStylesheetRule::Css(rule) = self.native_rule_at_path(rule_path)? else {
            return None;
        };
        let ancestor_path_len = rule_path.len().saturating_sub(1);
        Some(Self::rule_mutation_for_cascade(
            rule,
            self.css_rule_ancestors_at_path(&rule_path[..ancestor_path_len])?,
            change_kind,
        ))
    }

    fn rule_mutation_for_rule(
        &self,
        rule: CssRule,
        parent_path: &[usize],
        change_kind: RuleChangeKind,
    ) -> Option<LiveStylesheetRuleMutation> {
        Some(Self::rule_mutation_for_cascade(
            rule,
            self.css_rule_ancestors_at_path(parent_path)?,
            change_kind,
        ))
    }

    fn rule_mutation_for_cascade(
        rule: CssRule,
        ancestors: Vec<CssRule>,
        change_kind: RuleChangeKind,
    ) -> LiveStylesheetRuleMutation {
        if matches!(&rule, CssRule::NestedDeclarations(_))
            && let Some(style_index) = ancestors
                .iter()
                .rposition(|ancestor| matches!(ancestor, CssRule::Style(_)))
        {
            // Nested declarations inherit their selector from the nearest
            // enclosing style rule. Stylo therefore does not collect ordinary
            // element invalidations from the declaration rule itself. Report
            // the effective declaration change on that selector-bearing rule.
            return LiveStylesheetRuleMutation::new(
                ancestors[style_index].clone(),
                ancestors[..style_index].to_vec(),
                RuleChangeKind::StyleRuleDeclarations,
            );
        }
        LiveStylesheetRuleMutation::new(rule, ancestors, change_kind)
    }

    fn css_rule_ancestors_at_path(&self, path: &[usize]) -> Option<Vec<CssRule>> {
        let mut ancestors = Vec::with_capacity(path.len());
        for depth in 1..=path.len() {
            let NativeStylesheetRule::Css(rule) = self.native_rule_at_path(&path[..depth])? else {
                return None;
            };
            ancestors.push(rule);
        }
        Some(ancestors)
    }

    fn parse_declaration_block(
        &self,
        declaration_text: &str,
        rule_type: CssRuleType,
    ) -> PropertyDeclarationBlock {
        self.with_parser_context(rule_type, None, |context| {
            let mut input = ParserInput::new(declaration_text);
            let mut input = Parser::new(&mut input);
            parse_property_declaration_list(context, &mut input, &[])
        })
    }

    fn parse_style_rule_selectors(
        &self,
        selector_text: &str,
        containing_rule_type_bits: u32,
        parse_relative_rule_type: Option<CssRuleType>,
    ) -> Result<SelectorList<SelectorImpl>, CssRuleInsertError> {
        self.with_parser_context(CssRuleType::Style, parse_relative_rule_type, |context| {
            context.nesting_context = NestingContext::new(
                CssRuleTypes::from_bits(containing_rule_type_bits),
                parse_relative_rule_type,
            );
            let selector_parser = SelectorParser {
                stylesheet_origin: context.stylesheet_origin,
                namespaces: &context.namespaces,
                url_data: context.url_data,
                for_supports_rule: false,
            };
            let mut input = ParserInput::new(selector_text);
            let mut input = Parser::new(&mut input);
            input
                .parse_entirely(|input| {
                    SelectorList::parse(
                        &selector_parser,
                        input,
                        context.nesting_context.parse_relative,
                    )
                })
                .map_err(|_| CssRuleInsertError::Syntax)
        })
    }

    fn parse_font_face_descriptor_block(
        &self,
        descriptor_text: &str,
    ) -> Result<FontFaceRule, CssRuleInsertError> {
        self.with_parser_context(CssRuleType::FontFace, None, |context| {
            parse_font_face_cssom_rule_with_stylo_context(context, descriptor_text)
        })
    }

    fn parse_page_margin_descriptor_block(
        &self,
        rule_name: &str,
        descriptor_text: &str,
    ) -> Result<PropertyDeclarationBlock, CssRuleInsertError> {
        let rule =
            self.parse_detached_rule(&format!("@page {{ {rule_name} {{ {descriptor_text} }} }}"))?;
        let CssRule::Page(rule) = rule else {
            return Err(CssRuleInsertError::Syntax);
        };
        let guard = self.stylesheet.shared_lock.read();
        let rule = rule.read_with(&guard);
        let rules = rule.rules.read_with(&guard);
        let Some(CssRule::Margin(rule)) = rules.0.first() else {
            return Err(CssRuleInsertError::Syntax);
        };
        Ok(rule.block.read_with(&guard).clone())
    }

    fn parse_detached_rule(&self, rule_text: &str) -> Result<CssRule, CssRuleInsertError> {
        let contents = self.current_contents();
        let rules = CssRules::new(Vec::new(), &self.stylesheet.shared_lock);
        let guard = self.stylesheet.shared_lock.read();
        rules
            .read_with(&guard)
            .parse_rule_for_insert(
                &self.stylesheet.shared_lock,
                rule_text,
                &contents,
                0,
                CssRuleTypes::default(),
                None,
                None,
                AllowImportRules::No,
            )
            .map_err(css_rule_insert_error)
    }

    fn with_parser_context<R>(
        &self,
        rule_type: CssRuleType,
        parse_relative_rule_type: Option<CssRuleType>,
        f: impl FnOnce(&mut ParserContext<'_>) -> R,
    ) -> R {
        let contents = self.current_contents();
        let mut context = ParserContext::new(
            contents.origin,
            &contents.url_data,
            Some(rule_type),
            ParsingMode::DEFAULT,
            contents.quirks_mode,
            Cow::Borrowed(&contents.namespaces),
            None,
            None,
            AttrTaint::default(),
        );
        context.nesting_context =
            NestingContext::new(CssRuleTypes::from(rule_type), parse_relative_rule_type);
        f(&mut context)
    }

    fn parse_nested_rule_for_insert(
        &self,
        parent_path: &[usize],
        rule_text: &str,
        index: usize,
        containing_rule_type_bits: u32,
        parse_relative_rule_type: Option<CssRuleType>,
        replacing: bool,
    ) -> Result<CssRule, CssRuleInsertError> {
        let NativeStylesheetRule::Css(parent_rule) = self
            .native_rule_at_path(parent_path)
            .ok_or(CssRuleInsertError::HierarchyRequest)?
        else {
            return Err(CssRuleInsertError::HierarchyRequest);
        };
        let child_rules = self
            .existing_child_rules_for_rule(&parent_rule)
            .or_else(|| {
                matches!(parent_rule, CssRule::Style(_))
                    .then(|| CssRules::new(Vec::new(), &self.stylesheet.shared_lock))
            })
            .ok_or(CssRuleInsertError::HierarchyRequest)?;
        let contents = self.current_contents();
        let guard = self.stylesheet.shared_lock.read();
        let child_rules = child_rules.read_with(&guard);
        if replacing && index >= child_rules.0.len() {
            return Err(CssRuleInsertError::IndexSize);
        }
        child_rules
            .parse_rule_for_insert(
                &self.stylesheet.shared_lock,
                rule_text,
                &contents,
                index,
                CssRuleTypes::from_bits(containing_rule_type_bits),
                parse_relative_rule_type,
                None,
                AllowImportRules::No,
            )
            .map_err(css_rule_insert_error)
    }

    pub(super) fn current_contents(&self) -> ServoArc<StylesheetContents> {
        let guard = self.stylesheet.shared_lock.read();
        self.stylesheet.contents.read_with(&guard).clone()
    }

    pub(crate) fn top_level_rule_count(&self) -> usize {
        let guard = self.stylesheet.shared_lock.read();
        let contents = self.stylesheet.contents.read_with(&guard);
        contents.rules.read_with(&guard).0.len()
    }

    fn parse_top_level_rule_for_insert(
        &self,
        rule_text: &str,
        index: usize,
    ) -> Result<CssRule, CssRuleInsertError> {
        let import_loader = LiveStylesheetImportPlaceholderLoader;
        let loader = matches!(self.allow_import_rules, AllowImportRules::Yes)
            .then_some(&import_loader as &dyn StylesheetLoader);
        let guard = self.stylesheet.shared_lock.read();
        let contents = self.stylesheet.contents.read_with(&guard);
        let parsed = contents.rules.read_with(&guard).parse_rule_for_insert(
            &self.stylesheet.shared_lock,
            rule_text,
            contents,
            index,
            CssRuleTypes::default(),
            None,
            loader,
            self.allow_import_rules,
        );
        match parsed {
            Ok(rule) => Ok(rule),
            Err(RulesMutateError::HierarchyRequest)
                if css_text_starts_with_at_keyword(rule_text, "namespace")
                    && contents.rules.read_with(&guard).0.iter().any(|rule| {
                        !matches!(rule, CssRule::Import(_) | CssRule::Namespace(_))
                    }) =>
            {
                Err(CssRuleInsertError::InvalidState)
            }
            Err(error) => Err(css_rule_insert_error(error)),
        }
    }

    fn top_level_rule_is_namespace(&self, index: usize) -> Result<bool, CssRuleInsertError> {
        let guard = self.stylesheet.shared_lock.read();
        let contents = self.stylesheet.contents.read_with(&guard);
        contents
            .rules
            .read_with(&guard)
            .0
            .get(index)
            .map(|rule| matches!(rule, CssRule::Namespace(_)))
            .ok_or(CssRuleInsertError::IndexSize)
    }

    pub(crate) fn replace_from_text(&self, css_text: &str) {
        self.shared_initial_contents.borrow_mut().take();
        let shared_lock = self.stylesheet.shared_lock.clone();
        let replacement = ServoArc::new(parse_live_stylesheet(
            css_text,
            &self.base_url,
            self.stylesheet.media.clone(),
            shared_lock.clone(),
            self.quirks_mode,
            self.allow_import_rules,
        ));
        let replacement_contents = {
            let guard = replacement.shared_lock.read();
            replacement.contents.read_with(&guard).clone()
        };
        // A whole-sheet replacement creates a new rule tree. Retained CSSRule
        // wrappers continue to expose the old native rules until the V8 layer
        // snapshots and releases them; they must never be rebound by matching
        // cssText against the replacement tree.
        self.retire_all_rule_wrapper_bindings();
        {
            let mut guard = shared_lock.write();
            *self.stylesheet.contents.write_with(&mut guard) = replacement_contents;
        }
        self.reconcile_import_edges();
        self.font_face_rule_identities.borrow_mut().clear();
        self.note_contents_mutation();
    }

    pub(crate) fn note_contents_mutation(&self) {
        self.contents_revision
            .set(self.contents_revision.get().saturating_add(1));
        self.note_cascade_mutation();
        self.derived_state.clear_serialized_css_text();
        self.derived_state.clear_dependency_summary();
        self.font_face_cache.borrow_mut().take();
    }

    pub(super) fn note_cascade_mutation(&self) {
        self.note_import_descendant_mutation();
    }
}
