// Minimal Obsidian API mock for Jest tests.
// Only exposes what the plugin actually uses.

export class Plugin {
  app: App;
  settings: unknown = {};
  constructor(app: App) { this.app = app; }
  addRibbonIcon() { return document.createElement("div"); }
  addStatusBarItem() { return document.createElement("span"); }
  addCommand() {}
  addSettingTab() {}
  registerEvent() {}
  registerInterval() {}
  registerObsidianProtocolHandler() {}
  async loadData() { return {}; }
  async saveData() {}
}

export class PluginSettingTab {
  app: App;
  containerEl: HTMLElement;
  constructor(app: App) {
    this.app = app;
    this.containerEl = document.createElement("div");
  }
  display() {}
}

export class Setting {
  constructor(_containerEl: HTMLElement) {}
  setName(_name: string) { return this; }
  setDesc(_desc: string) { return this; }
  addText(_cb: (t: TextComponent) => unknown) { return this; }
  addButton(_cb: (b: ButtonComponent) => unknown) { return this; }
  addDropdown(_cb: (d: DropdownComponent) => unknown) { return this; }
  addToggle(_cb: (t: ToggleComponent) => unknown) { return this; }
}

class TextComponent {
  inputEl: HTMLInputElement;
  constructor() { this.inputEl = document.createElement("input"); }
  setPlaceholder(_p: string) { return this; }
  setValue(_v: string) { return this; }
  onChange(_cb: (v: string) => unknown) { return this; }
}

class ButtonComponent {
  setButtonText(_t: string) { return this; }
  setCta() { return this; }
  onClick(_cb: () => unknown) { return this; }
}

class DropdownComponent {
  addOption(_val: string, _label: string) { return this; }
  setValue(_v: string) { return this; }
  onChange(_cb: (v: string) => unknown) { return this; }
}

class ToggleComponent {
  setValue(_v: boolean) { return this; }
  onChange(_cb: (v: boolean) => unknown) { return this; }
}

export class Notice {
  constructor(public message: string) {}
}

export class App {
  vault = {
    on: () => {},
    adapter: {
      read: async () => "",
      write: async () => {},
      exists: async () => false,
      list: async () => ({ files: [], folders: [] }),
      remove: async () => {},
    },
    createFolder: async () => {},
  };
  workspace = {
    getActiveFile: () => null,
  };
}

export class TFile {
  path = "";
  basename = "";
}

export class MarkdownView {}

export const debounce = (fn: (...args: unknown[]) => unknown, _ms: number) => fn;
