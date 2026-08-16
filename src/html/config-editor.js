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
          secret: metadata.secret || metadata.children?.[objectKey]?.secret,
          label: metadata.childLabels?.[objectKey] || metadata.children?.[objectKey]?.label || objectKey
        };
        const row = document.createElement("div");
        row.className = `collection-row${isObject(childValue) || Array.isArray(childValue) ? " complex" : ""}`;
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
          key.setAttribute("aria-label", "配置键");
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
          this.form.querySelector(`.config-field[data-path="${path[0]}"] .collection-key-input:last-of-type`)?.focus();
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
