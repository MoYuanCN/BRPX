(() => {
  const clone = value => JSON.parse(JSON.stringify(value));
  const isObject = value => value !== null && typeof value === "object" && !Array.isArray(value);

  class BRPXConfigEditor {
    constructor(options) {
      this.form = options.form;
      this.formPanel = options.formPanel;
      this.jsonPanel = options.jsonPanel;
      this.jsonEditor = options.jsonEditor;
      this.search = options.search;
      this.count = options.count;
      this.formatButton = options.formatButton;
      this.modeButtons = options.modeButtons;
      this.onError = options.onError || (() => {});
      this.onChange = options.onChange || (() => {});
      this.mode = "form";
      this.draft = {};
      this.schema = { groups: [] };

      this.search?.addEventListener("input", () => this.applyFilter());
      this.modeButtons.forEach(button => {
        button.addEventListener("click", () => this.setMode(button.dataset.configMode));
      });
    }

    load(config, schema) {
      this.draft = clone(config);
      this.schema = schema;
      this.jsonEditor.value = JSON.stringify(this.draft, null, 2);
      this.render();
      this.updateMode();
    }

    getValue() {
      if (this.mode === "json") {
        this.draft = this.parseJson();
      }
      return clone(this.draft);
    }

    setMode(mode) {
      if (mode === this.mode) return true;
      if (mode === "form") {
        try {
          this.draft = this.parseJson();
          this.render();
        } catch (error) {
          this.onError(error.message);
          return false;
        }
      } else {
        this.syncJson();
      }
      this.mode = mode;
      this.updateMode();
      return true;
    }

    formatJson() {
      try {
        this.draft = this.parseJson();
        this.syncJson();
      } catch (error) {
        this.onError(error.message);
      }
    }

    showErrors(fields = []) {
      this.form.querySelectorAll(".config-field.invalid").forEach(field => {
        field.classList.remove("invalid");
        field.querySelector(".field-validation")?.remove();
      });
      if (!fields.length) return;
      this.setMode("form");
      let first = null;
      fields.forEach(error => {
        const rootPath = String(error.field || "").split(".")[0];
        const field = [...this.form.querySelectorAll(".config-field")]
          .find(candidate => candidate.dataset.path === rootPath);
        if (!field) return;
        field.classList.add("invalid");
        const message = document.createElement("p");
        message.className = "field-validation";
        message.textContent = `${error.field}: ${error.message}`;
        field.querySelector(".config-field-info").append(message);
        first ||= field;
      });
      first?.scrollIntoView({ behavior: "smooth", block: "center" });
    }

    parseJson() {
      const value = JSON.parse(this.jsonEditor.value);
      if (!isObject(value)) throw new Error("运行配置必须是 JSON 对象");
      return value;
    }

    syncJson() {
      this.jsonEditor.value = JSON.stringify(this.draft, null, 2);
    }

    updateMode() {
      const formMode = this.mode === "form";
      this.formPanel.classList.toggle("hidden", !formMode);
      this.jsonPanel.classList.toggle("hidden", formMode);
      this.formatButton.classList.toggle("hidden", formMode);
      this.modeButtons.forEach(button => {
        const active = button.dataset.configMode === this.mode;
        button.classList.toggle("active", active);
        button.setAttribute("aria-pressed", String(active));
      });
      if (formMode) this.applyFilter();
    }

    render() {
      this.form.replaceChildren();
      const covered = new Set();
      this.schema.groups.forEach(group => {
        const section = this.renderGroup(group);
        group.fields.forEach(field => covered.add(field.path));
        this.form.append(section);
      });

      const unknown = Object.keys(this.draft).filter(path => !covered.has(path));
      if (unknown.length) {
        this.form.append(this.renderGroup({
          id: "compatibility",
          label: "兼容字段",
          description: "当前程序返回但说明清单尚未收录的字段，仍可通过通用控件编辑。",
          fields: unknown.map(path => ({
            path,
            label: path,
            description: "兼容性配置字段。",
            effect: "保存前请确认该字段对应的版本行为。"
          }))
        }));
      }
      this.syncJson();
      this.applyFilter();
      if (window.lucide) window.lucide.createIcons();
    }

    renderGroup(group) {
      const section = document.createElement("section");
      section.className = "config-section";
      section.dataset.group = group.id;

      const header = document.createElement("header");
      header.className = "config-section-header";
      const title = document.createElement("h3");
      title.textContent = group.label;
      const description = document.createElement("p");
      description.textContent = group.description;
      header.append(title, description);

      const body = document.createElement("div");
      body.className = "config-section-body";
      group.fields.forEach(field => body.append(this.renderField(field, group)));
      section.append(header, body);
      return section;
    }

    renderField(field, group) {
      const path = field.path.split(".");
      const value = this.getAtPath(path);
      const wrapper = document.createElement("div");
      wrapper.className = `config-field${field.danger ? " danger-field" : ""}`;
      wrapper.dataset.path = field.path;
      wrapper.dataset.search = [group.label, field.label, field.path, field.description, field.effect]
        .filter(Boolean).join(" ").toLocaleLowerCase("zh-CN");

      const info = document.createElement("div");
      info.className = "config-field-info";
      const heading = document.createElement("div");
      heading.className = "config-field-heading";
      const label = document.createElement("strong");
      label.textContent = field.label;
      const key = document.createElement("code");
      key.textContent = field.path;
      heading.append(label, key);
      if (field.restart) heading.append(this.badge("重启生效", "amber"));
      else heading.append(this.badge("立即生效", "green"));
      if (field.secret) heading.append(this.badge("敏感", "red"));

      const description = document.createElement("p");
      description.textContent = field.description;
      const effect = document.createElement("p");
      effect.className = "config-effect";
      effect.textContent = field.effect;
      info.append(heading, description, effect);

      const control = document.createElement("div");
      control.className = "config-control";
      control.append(this.renderValue(path, value, field));
      wrapper.append(info, control);
      return wrapper;
    }

    renderValue(path, value, metadata = {}) {
      if (metadata.control === "variant") return this.renderVariant(path, value, metadata);
      if (metadata.control === "json") return this.renderSerializedJson(path, value, metadata);
      if (Array.isArray(value)) return this.renderArray(path, value, metadata);
      if (isObject(value)) return this.renderObject(path, value, metadata);
      return this.renderPrimitive(path, value, metadata);
    }

    renderPrimitive(path, value, metadata = {}) {
      if (typeof value === "boolean") {
        const label = document.createElement("label");
        label.className = "switch";
        const input = document.createElement("input");
        input.type = "checkbox";
        input.checked = value;
        input.disabled = Boolean(metadata.readonly);
        input.setAttribute("aria-label", metadata.label || path.at(-1));
        const track = document.createElement("span");
        track.className = "switch-track";
        const state = document.createElement("span");
        state.className = "switch-state";
        state.textContent = value ? "启用" : "停用";
        input.addEventListener("change", () => {
          this.setAtPath(path, input.checked);
          state.textContent = input.checked ? "启用" : "停用";
          this.changed();
        });
        label.append(input, track, state);
        return label;
      }

      if (metadata.control === "select") {
        const select = document.createElement("select");
        select.setAttribute("aria-label", metadata.label || path.at(-1));
        metadata.options.forEach(option => {
          const item = document.createElement("option");
          item.value = option;
          item.textContent = option;
          item.selected = option === value;
          select.append(item);
        });
        select.addEventListener("change", () => {
          this.setAtPath(path, select.value);
          this.changed();
        });
        return select;
      }

      const multiline = metadata.control === "textarea" || (typeof value === "string" && value.length > 180);
      const input = document.createElement(multiline ? "textarea" : "input");
      if (multiline) {
        input.rows = 5;
      } else if (typeof value === "number") {
        input.type = "number";
        input.step = "1";
        if (metadata.min !== undefined) input.min = metadata.min;
        if (metadata.max !== undefined) input.max = metadata.max;
      } else {
        input.type = metadata.secret ? "password" : "text";
        if (metadata.control === "url") input.inputMode = "url";
        input.autocomplete = "off";
      }
      input.value = value ?? "";
      input.disabled = Boolean(metadata.readonly);
      input.setAttribute("aria-label", metadata.label || path.at(-1));
      input.addEventListener("input", () => {
        if (typeof value === "number") {
          if (input.value === "") return;
          this.setAtPath(path, Number(input.value));
        } else {
          this.setAtPath(path, input.value);
        }
        this.changed();
      });
      if (typeof value === "number") {
        input.addEventListener("change", () => {
          if (input.value !== "") return;
          input.value = "0";
          this.setAtPath(path, 0);
          this.changed();
        });
      }
      return input;
    }

    renderSerializedJson(path, value, metadata) {
      const wrapper = document.createElement("div");
      wrapper.className = "serialized-json-editor";
      const visual = document.createElement("details");
      visual.className = "json-mode";
      const visualSummary = document.createElement("summary");
      const tree = document.createElement("div");
      tree.className = "json-tree-root";
      visual.append(visualSummary, tree);

      const rawMode = document.createElement("details");
      rawMode.className = "json-mode raw-json-mode";
      const rawSummary = document.createElement("summary");
      rawSummary.textContent = "原始 JSON";
      const raw = document.createElement("textarea");
      raw.className = "json-raw-input";
      raw.rows = 10;
      raw.spellcheck = false;
      raw.setAttribute("aria-label", `${metadata.label || path.at(-1)} 原始 JSON`);
      const validation = document.createElement("p");
      validation.className = "json-validation hidden";
      rawMode.append(rawSummary, raw, validation);

      let parsed = null;
      let treeRendered = false;
      const parse = text => {
        try {
          const next = JSON.parse(text);
          if (metadata.jsonRoot === "object" && !isObject(next)) {
            return { error: "JSON 根节点必须是对象" };
          }
          return { value: next };
        } catch (error) {
          return { error: `JSON 无法解析：${error.message}` };
        }
      };
      const updateSummary = () => {
        const count = isObject(parsed) ? Object.keys(parsed).length : Array.isArray(parsed) ? parsed.length : 1;
        visualSummary.textContent = `可视化编辑 · ${count} 项`;
      };
      const showValidation = message => {
        validation.textContent = message || "";
        validation.classList.toggle("hidden", !message);
        raw.classList.toggle("invalid-input", Boolean(message));
      };
      const commit = () => {
        const serialized = JSON.stringify(parsed);
        this.setAtPath(path, serialized);
        raw.value = JSON.stringify(parsed, null, 2);
        showValidation("");
        updateSummary();
        this.changed();
      };
      const renderTree = () => {
        tree.replaceChildren(this.renderJsonNode(
          parsed,
          next => { parsed = next; },
          commit,
          renderTree,
          0,
          metadata.jsonRoot === "object"
        ));
        treeRendered = true;
        if (window.lucide) window.lucide.createIcons();
      };
      const initial = parse(String(value ?? ""));
      if (initial.error) {
        visual.classList.add("hidden");
        rawMode.open = true;
        raw.value = String(value ?? "");
        showValidation(initial.error);
      } else {
        parsed = initial.value;
        raw.value = JSON.stringify(parsed, null, 2);
        updateSummary();
      }
      visual.addEventListener("toggle", () => {
        if (visual.open && !treeRendered && parsed !== null) renderTree();
      });
      raw.addEventListener("input", () => {
        const result = parse(raw.value);
        if (result.error) {
          this.setAtPath(path, raw.value);
          visual.classList.add("hidden");
          showValidation(result.error);
          this.changed();
          return;
        }
        parsed = result.value;
        treeRendered = false;
        tree.replaceChildren();
        visual.classList.remove("hidden");
        showValidation("");
        updateSummary();
        this.setAtPath(path, JSON.stringify(parsed));
        this.changed();
        if (visual.open) renderTree();
      });
      wrapper.append(visual, rawMode);
      return wrapper;
    }

    renderJsonNode(value, update, commit, refresh, depth, typeLocked = false) {
      if (isObject(value) || Array.isArray(value)) {
        const details = document.createElement("details");
        details.className = "json-container";
        details.open = depth === 0;
        const summary = document.createElement("summary");
        const isArray = Array.isArray(value);
        const size = isArray ? value.length : Object.keys(value).length;
        summary.textContent = `${isArray ? "数组" : "对象"} · ${size} 项`;
        const body = document.createElement("div");
        body.className = "json-container-body";
        if (!typeLocked) {
          body.append(this.renderJsonTypeSelect(isArray ? "array" : "object", update, commit, refresh));
        }

        const entries = isArray ? value.map((item, index) => [String(index), item]) : Object.entries(value);
        entries.forEach(([key, child], index) => {
          const row = document.createElement("div");
          row.className = "json-tree-row";
          let keyControl;
          if (isArray) {
            keyControl = document.createElement("span");
            keyControl.className = "json-index";
            keyControl.textContent = `项目 ${index + 1}`;
          } else {
            keyControl = document.createElement("input");
            keyControl.className = "json-key-input";
            keyControl.value = key;
            keyControl.setAttribute("aria-label", `JSON 字段 ${key}`);
            keyControl.addEventListener("change", () => {
              const nextKey = keyControl.value.trim();
              if (!nextKey || (nextKey !== key && Object.hasOwn(value, nextKey))) {
                this.onError(nextKey ? "JSON 字段名已经存在" : "JSON 字段名不能为空");
                refresh();
                return;
              }
              if (nextKey === key) return;
              const renamed = {};
              Object.entries(value).forEach(([currentKey, currentValue]) => {
                renamed[currentKey === key ? nextKey : currentKey] = currentValue;
              });
              update(renamed);
              commit();
              refresh();
            });
          }
          const childControl = this.renderJsonNode(
            child,
            next => {
              if (isArray) value[index] = next;
              else value[key] = next;
            },
            commit,
            refresh,
            depth + 1,
            false
          );
          const remove = this.iconButton("trash-2", "删除 JSON 项", () => {
            if (isArray) value.splice(index, 1);
            else delete value[key];
            commit();
            refresh();
          });
          row.append(keyControl, childControl, remove);
          body.append(row);
        });
        body.append(this.addButton(isArray ? "添加数组项" : "添加字段", () => {
          if (isArray) {
            value.push("");
          } else {
            let index = 1;
            while (Object.hasOwn(value, `new_field_${index}`)) index += 1;
            value[`new_field_${index}`] = "";
          }
          commit();
          refresh();
        }));
        details.append(summary, body);
        return details;
      }

      const wrapper = document.createElement("div");
      wrapper.className = "json-scalar";
      const type = value === null ? "null" : typeof value;
      const typeSelect = this.renderJsonTypeSelect(type, update, commit, refresh);

      let input;
      if (type === "boolean") {
        const label = document.createElement("label");
        label.className = "switch json-boolean";
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.checked = value;
        checkbox.setAttribute("aria-label", "JSON 布尔值");
        const track = document.createElement("span");
        track.className = "switch-track";
        const state = document.createElement("span");
        state.className = "switch-state";
        state.textContent = value ? "true" : "false";
        checkbox.addEventListener("change", () => {
          update(checkbox.checked);
          state.textContent = checkbox.checked ? "true" : "false";
          commit();
        });
        label.append(checkbox, track, state);
        input = label;
      } else if (type === "null") {
        input = document.createElement("span");
        input.className = "json-null";
        input.textContent = "null";
      } else {
        const multiline = type === "string" && (value.length > 100 || value.includes("\n"));
        input = document.createElement(multiline ? "textarea" : "input");
        if (multiline) input.rows = 3;
        if (type === "number") {
          input.type = "number";
          input.step = "any";
        }
        input.value = value;
        input.setAttribute("aria-label", "JSON 值");
        input.addEventListener("input", () => {
          if (type === "number" && input.value === "") return;
          update(type === "number" ? Number(input.value) : input.value);
          commit();
        });
      }
      wrapper.append(typeSelect, input);
      return wrapper;
    }

    renderJsonTypeSelect(type, update, commit, refresh) {
      const typeSelect = document.createElement("select");
      typeSelect.className = "json-type-select";
      typeSelect.setAttribute("aria-label", "JSON 类型");
      ["string", "number", "boolean", "null", "object", "array"].forEach(option => {
        const item = document.createElement("option");
        item.value = option;
        item.textContent = { string: "文本", number: "数字", boolean: "布尔", null: "空值", object: "对象", array: "数组" }[option];
        item.selected = option === type;
        typeSelect.append(item);
      });
      typeSelect.addEventListener("change", () => {
        const defaults = { string: "", number: 0, boolean: false, null: null, object: {}, array: [] };
        update(clone(defaults[typeSelect.value]));
        commit();
        refresh();
      });
      return typeSelect;
    }

    renderArray(path, value, metadata) {
      const wrapper = document.createElement("div");
      wrapper.className = "collection-editor";
      value.forEach((item, index) => {
        const row = document.createElement("div");
        row.className = "collection-row array-row";
        const label = document.createElement("span");
        label.className = "collection-key";
        label.textContent = `${metadata.itemLabel || "项目"} ${index + 1}`;
        const control = this.renderValue([...path, index], item, metadata.item || {});
        row.append(label, control, this.iconButton("trash-2", "删除", () => {
          value.splice(index, 1);
          this.render();
          this.changed();
        }));
        wrapper.append(row);
      });
      wrapper.append(this.addButton("添加项目", () => {
        value.push(clone(metadata.itemDefault ?? ""));
        this.render();
        this.changed();
      }));
      return wrapper;
    }

    renderObject(path, value, metadata) {
      const wrapper = document.createElement("div");
      wrapper.className = "collection-editor object-editor";
      Object.entries(value).forEach(([objectKey, childValue]) => {
        const childMetadata = {
          ...(metadata.children?.[objectKey] || {}),
          control: metadata.children?.[objectKey]?.control || metadata.valueControl,
          options: metadata.children?.[objectKey]?.options || metadata.valueOptions,
          secret: metadata.secret || metadata.children?.[objectKey]?.secret,
          label: metadata.childLabels?.[objectKey] || metadata.children?.[objectKey]?.label || objectKey
        };
        const row = document.createElement("div");
        row.className = `collection-row${isObject(childValue) || Array.isArray(childValue) || childMetadata.control === "json" ? " complex" : ""}`;
        if (metadata.fixedKeys) {
          const keyInfo = document.createElement("div");
          keyInfo.className = "collection-key-info";
          const key = document.createElement("span");
          key.className = "collection-key";
          key.textContent = childMetadata.label;
          keyInfo.append(key);
          if (childMetadata.description) {
            const help = document.createElement("small");
            help.textContent = childMetadata.description;
            keyInfo.append(help);
          }
          row.append(keyInfo);
        } else {
          const key = document.createElement("input");
          key.className = "collection-key-input";
          key.value = objectKey;
          key.placeholder = metadata.keyPlaceholder || "";
          key.setAttribute("aria-label", metadata.keyLabel || "配置键");
          key.addEventListener("change", () => this.renameObjectKey(path, objectKey, key.value));
          row.append(key);
        }
        row.append(this.renderValue([...path, objectKey], childValue, childMetadata));
        if (!metadata.fixedKeys) {
          row.append(this.iconButton("trash-2", "删除", () => {
            delete value[objectKey];
            this.render();
            this.changed();
          }));
        }
        wrapper.append(row);
      });
      if (!metadata.fixedKeys) {
        wrapper.append(this.addButton("添加条目", () => {
          const key = this.nextObjectKey(value);
          value[key] = this.defaultObjectValue(metadata, value);
          this.render();
          const inputs = this.form.querySelectorAll(`.config-field[data-path="${path[0]}"] .collection-key-input`);
          const input = inputs.item(inputs.length - 1);
          input?.focus();
          input?.select();
          this.changed();
        }));
      }
      return wrapper;
    }

    renderVariant(path, value, metadata) {
      const wrapper = document.createElement("div");
      wrapper.className = "variant-editor";
      const current = Object.keys(value || {})[0] || Object.keys(metadata.variants)[0];
      const select = document.createElement("select");
      select.setAttribute("aria-label", "通知渠道");
      Object.entries(metadata.variants).forEach(([key, variant]) => {
        const option = document.createElement("option");
        option.value = key;
        option.textContent = variant.label;
        option.selected = key === current;
        select.append(option);
      });
      select.addEventListener("change", () => {
        const variant = metadata.variants[select.value];
        this.setAtPath(path, { [select.value]: clone(variant.default) });
        this.render();
        this.changed();
      });
      const selected = metadata.variants[current];
      const selectedValue = value?.[current] ?? clone(selected.default);
      wrapper.append(select, this.renderObject([...path, current], selectedValue, {
        fixedKeys: true,
        children: selected.children
      }));
      return wrapper;
    }

    renameObjectKey(path, oldKey, nextKey) {
      const value = this.getAtPath(path);
      const normalized = nextKey.trim();
      if (!normalized || (normalized !== oldKey && Object.hasOwn(value, normalized))) {
        this.onError(normalized ? "配置键已经存在" : "配置键不能为空");
        this.render();
        return;
      }
      if (normalized === oldKey) return;
      value[normalized] = value[oldKey];
      delete value[oldKey];
      this.render();
      this.changed();
    }

    defaultObjectValue(metadata, value) {
      if (metadata.valueControl === "select") return metadata.valueOptions?.[0] || "";
      if (metadata.valueControl === "json") return "{}";
      if (metadata.valueType === "number") return 0;
      if (metadata.valueType === "boolean") return false;
      const sample = Object.values(value)[0];
      if (typeof sample === "number") return 0;
      if (typeof sample === "boolean") return false;
      if (Array.isArray(sample)) return [];
      if (isObject(sample)) return {};
      return "";
    }

    nextObjectKey(value) {
      let index = 1;
      while (Object.hasOwn(value, `new_key_${index}`)) index += 1;
      return `new_key_${index}`;
    }

    changed() {
      this.syncJson();
      this.onChange(clone(this.draft));
    }

    getAtPath(path) {
      return path.reduce((value, key) => value?.[key], this.draft);
    }

    setAtPath(path, nextValue) {
      const parent = path.slice(0, -1).reduce((value, key) => value[key], this.draft);
      parent[path.at(-1)] = nextValue;
    }

    applyFilter() {
      const query = (this.search?.value || "").trim().toLocaleLowerCase("zh-CN");
      let visible = 0;
      let total = 0;
      this.form.querySelectorAll(".config-section").forEach(section => {
        let sectionVisible = 0;
        section.querySelectorAll(".config-field").forEach(field => {
          total += 1;
          const show = !query || field.dataset.search.includes(query);
          field.classList.toggle("hidden", !show);
          if (show) {
            visible += 1;
            sectionVisible += 1;
          }
        });
        section.classList.toggle("hidden", sectionVisible === 0);
      });
      if (this.count) this.count.textContent = `${visible} / ${total} 项`;
    }

    badge(text, color) {
      const badge = document.createElement("span");
      badge.className = `badge ${color}`;
      badge.textContent = text;
      return badge;
    }

    iconButton(icon, title, handler) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "icon-button";
      button.title = title;
      button.setAttribute("aria-label", title);
      const glyph = document.createElement("i");
      glyph.dataset.lucide = icon;
      button.append(glyph);
      button.addEventListener("click", handler);
      return button;
    }

    addButton(label, handler) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "button secondary collection-add";
      const icon = document.createElement("i");
      icon.dataset.lucide = "plus";
      const text = document.createElement("span");
      text.textContent = label;
      button.append(icon, text);
      button.addEventListener("click", handler);
      return button;
    }
  }

  window.BRPXConfigEditor = BRPXConfigEditor;
})();
