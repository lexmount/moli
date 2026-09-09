#!/usr/bin/env python

# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

import os
import sys

import toml

THIS_DIR = os.path.dirname(__file__)
sys.path.insert(0, os.path.join(os.path.dirname(THIS_DIR), "properties"))

import data


class Pseudo(object):
    def __init__(self, name, argument, enabled_in, pref):
        self.name = name
        self.camel = data.to_camel_case(name)
        self.capitalized = self.camel[0].upper() + self.camel[1:]
        self.argument = argument
        self.enabled_in = enabled_in
        self.pref = pref

    def is_pseudo_element(self):
        return isinstance(self, PseudoElement)

    def is_anon_box(self):
        return isinstance(self, AnonBox)

    def is_non_inheriting_anon_box(self):
        return isinstance(self, AnonBox) and not self.inherits

    def is_inheriting_anon_box(self):
        return isinstance(self, AnonBox) and self.inherits

    def flags(self):
        flags = []
        if self.enabled_in == "ua":
            flags.append("ENABLED_IN_UA")
        elif self.enabled_in == "chrome":
            flags.append("ENABLED_IN_UA")
            flags.append("ENABLED_IN_CHROME")
        if self.pref:
            flags.append("ENABLED_BY_PREF")
        if isinstance(self, PseudoElement):
            flags.append("IS_PSEUDO_ELEMENT")
            if self.is_css2:
                flags.append("IS_CSS2")
            if self.is_eager:
                flags.append("IS_EAGER")
            if self.is_js_created_nac:
                flags.append("IS_JS_CREATED_NAC")
            if self.is_flex_or_grid_item:
                flags.append("IS_FLEX_OR_GRID_ITEM")
            if self.is_element_backed:
                flags.append("IS_ELEMENT_BACKED")
            if self.is_tree_abiding:
                flags.append("IS_TREE_ABIDING")
            if self.parses_as_element_backed:
                flags.append("PARSES_AS_ELEMENT_BACKED")
            if self.supports_user_action_state:
                flags.append("SUPPORTS_USER_ACTION_STATE")
        if isinstance(self, AnonBox):
            if self.inherits:
                flags.append("IS_INHERITING_ANON_BOX")
            else:
                flags.append("IS_NON_INHERITING_ANON_BOX")
            if self.wrapper:
                flags.append("IS_WRAPPER_ANON_BOX")
        return flags


class AnonBox(Pseudo):
    def __init__(self, name, wrapper=False, inherits=True):
        super().__init__(name, argument=None, enabled_in="ua", pref=None)
        self.wrapper = wrapper
        self.inherits = inherits


class PseudoElement(Pseudo):
    def __init__(
        self,
        name,
        enabled_in="content",
        is_css2=False,
        is_eager=False,
        is_js_created_nac=False,
        is_flex_or_grid_item=False,
        is_element_backed=False,
        is_tree_abiding=False,
        parses_as_element_backed=None,
        supports_user_action_state=False,
        disabled_domains_pref=None,
        pref=None,
        argument=None,
    ):
        super().__init__(name, argument=argument, enabled_in=enabled_in, pref=pref)
        self.is_css2 = is_css2
        self.is_eager = is_eager
        self.is_js_created_nac = is_js_created_nac
        self.is_flex_or_grid_item = is_flex_or_grid_item
        self.is_element_backed = is_element_backed
        self.is_tree_abiding = is_element_backed or is_tree_abiding
        self.parses_as_element_backed = (
            parses_as_element_backed
            if parses_as_element_backed is not None
            else is_element_backed
        )
        self.supports_user_action_state = supports_user_action_state
        self.disabled_domains_pref = disabled_domains_pref


class PseudoElementData:
    def __init__(self):
        this_dir = os.path.dirname(__file__)
        pseudo_elements_toml = os.path.join(this_dir, "pseudo_elements.toml")
        anon_boxes_toml = os.path.join(this_dir, "anon_boxes.toml")
        self.anon_boxes = sorted(
            (
                AnonBox(name, **val)
                for name, val in toml.loads(open(anon_boxes_toml).read()).items()
            ),
            key=lambda n: n.inherits,
        )
        self.pseudo_elements = [
            PseudoElement(name, **val)
            for name, val in toml.loads(open(pseudo_elements_toml).read()).items()
        ]
        self.path_dependencies = [pseudo_elements_toml, anon_boxes_toml]

    def all_pseudos(self):
        return self.anon_boxes + self.pseudo_elements
