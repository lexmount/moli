(function (global) {
  "use strict";

  function stripComments(idl) {
    return idl.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
  }

  function splitTopLevel(input, separator) {
    const parts = [];
    let start = 0;
    let depth = 0;
    for (let index = 0; index < input.length; index += 1) {
      const char = input[index];
      if (char === "(" || char === "<" || char === "[") {
        depth += 1;
      } else if (char === ")" || char === ">" || char === "]") {
        depth = Math.max(0, depth - 1);
      } else if (char === separator && depth === 0) {
        parts.push(input.slice(start, index));
        start = index + 1;
      }
    }
    parts.push(input.slice(start));
    return parts;
  }

  function stripExtendedAttributes(member) {
    let output = member.trim();
    while (output.startsWith("[")) {
      const end = output.indexOf("]");
      if (end === -1) {
        break;
      }
      output = output.slice(end + 1).trim();
    }
    return output;
  }

  function leadingExtendedAttributeNames(member) {
    let output = member.trim();
    const names = new Set();
    while (output.startsWith("[")) {
      const end = output.indexOf("]");
      if (end === -1) {
        break;
      }
      const content = output.slice(1, end);
      for (const rawAttribute of splitTopLevel(content, ",")) {
        const match = rawAttribute.trim().match(/^([A-Za-z_][A-Za-z0-9_]*)/);
        if (match) {
          names.add(match[1]);
        }
      }
      output = output.slice(end + 1).trim();
    }
    return names;
  }

  function requiredArgumentCount(args) {
    if (!args.trim()) {
      return 0;
    }
    let count = 0;
    for (const rawArg of splitTopLevel(args, ",")) {
      const arg = stripExtendedAttributes(rawArg).trim();
      if (!arg || arg.startsWith("optional ") || arg.includes(" = ") || arg.includes("...")) {
        continue;
      }
      count += 1;
    }
    return count;
  }

  function emptyInterface(name, parent) {
    return {
      name,
      parent,
      attributes: [],
      constructors: [],
      operations: [],
      staticOperations: [],
      stringifier: false,
      stringifierAttribute: null,
    };
  }

  function addUnique(list, entry) {
    if (!list.some((item) => item.name === entry.name)) {
      list.push(entry);
    }
  }

  function parseMember(iface, rawMember) {
    const extendedAttributes = leadingExtendedAttributeNames(rawMember);
    let member = stripExtendedAttributes(rawMember);
    let stringifierAttribute = false;
    if (!member) {
      return;
    }

    const constructor = member.match(/^constructor\s*\(([\s\S]*)\)$/);
    if (constructor) {
      iface.constructors.push({
        length: requiredArgumentCount(constructor[1]),
      });
      return;
    }

    if (member.startsWith("stringifier")) {
      iface.stringifier = true;
      stringifierAttribute = true;
      member = member.slice("stringifier".length).trim();
      if (!member) {
        return;
      }
    }

    const staticOperation = member.match(
      /^static\s+[\s\S]+?\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([\s\S]*)\)$/,
    );
    if (staticOperation) {
      addUnique(iface.staticOperations, {
        name: staticOperation[1],
        length: requiredArgumentCount(staticOperation[2]),
        unforgeable: extendedAttributes.has("LegacyUnforgeable"),
      });
      return;
    }

    const operation = member.match(
      /^[\s\S]+?\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([\s\S]*)\)$/,
    );
    if (operation) {
      addUnique(iface.operations, {
        name: operation[1],
        length: requiredArgumentCount(operation[2]),
        unforgeable: extendedAttributes.has("LegacyUnforgeable"),
      });
      return;
    }

    const attribute = member.match(
      /^(readonly\s+)?attribute\s+[\s\S]+?\s+([A-Za-z_][A-Za-z0-9_]*)$/,
    );
    if (attribute) {
      addUnique(iface.attributes, {
        name: attribute[2],
        readonly: !!attribute[1],
        unforgeable: extendedAttributes.has("LegacyUnforgeable"),
      });
      if (stringifierAttribute) {
        iface.stringifierAttribute = attribute[2];
      }
    }
  }

  function parse(idl) {
    const interfaces = [];
    const byName = new Map();
    const interfacePattern =
      /(?:partial\s+)?interface\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*([A-Za-z_][A-Za-z0-9_]*))?\s*\{([\s\S]*?)\};/g;
    let interfaceMatch;
    const source = stripComments(idl);
    while ((interfaceMatch = interfacePattern.exec(source)) !== null) {
      const name = interfaceMatch[1];
      const parent = interfaceMatch[2] || null;
      const body = interfaceMatch[3];
      let iface = byName.get(name);
      if (!iface) {
        iface = emptyInterface(name, parent);
        byName.set(name, iface);
        interfaces.push(iface);
      } else if (!iface.parent && parent) {
        iface.parent = parent;
      }
      for (const member of splitTopLevel(body, ";")) {
        parseMember(iface, member);
      }
    }
    return interfaces;
  }

  global.WebIDLParser = { parse };
})(globalThis);
