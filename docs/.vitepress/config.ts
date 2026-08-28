import { defineConfig } from "vitepress";

// Project site on GitHub Pages: https://tuls-rs.github.io/tuls/
const base = "/tuls/";

const nav = [
  { text: "Home", link: "/" },
  {
    text: "Guide",
    items: [
      { text: "Getting started", link: "/guide/getting-started" },
      { text: "Quick start", link: "/guide/quick-start" },
      { text: "Connecting MCP clients", link: "/guide/connecting-clients" },
      { text: "CLI reference", link: "/guide/cli-reference" },
      { text: "Capability policy", link: "/guide/capability-policy" },
    ],
  },
  {
    text: "Servers",
    items: [
      { text: "Overview", link: "/servers/" },
      { text: "Filesystem", link: "/servers/filesystem" },
      { text: "Fetch", link: "/servers/fetch" },
      { text: "Memory", link: "/servers/memory" },
      { text: "Shell", link: "/servers/shell" },
      { text: "Skills", link: "/servers/skills" },
      { text: "Agents", link: "/servers/agents" },
    ],
  },
  {
    text: "Configuration",
    items: [
      { text: "Subagent configuration", link: "/configuration/subagents" },
      { text: "Provider configuration", link: "/configuration/providers" },
      { text: "OpenRouter subagents", link: "/configuration/openrouter" },
      { text: "Child MCP servers", link: "/configuration/child-mcp" },
      { text: "Agent profiles", link: "/configuration/agent-profiles" },
    ],
  },
  {
    text: "Concepts",
    items: [
      { text: "Security model", link: "/concepts/security-model" },
      { text: "Limits & bounded behavior", link: "/concepts/limits" },
      { text: "Naming conventions", link: "/concepts/naming" },
      { text: "Workspace layout", link: "/concepts/workspace-layout" },
      {
        text: "Least-privilege example",
        link: "/concepts/least-privilege-example",
      },
    ],
  },
  { text: "Troubleshooting", link: "/troubleshooting" },
  { text: "Development", link: "/development" },
];

const guideSidebar = [
  {
    text: "Guide",
    items: [
      { text: "Getting started", link: "/guide/getting-started" },
      { text: "Quick start", link: "/guide/quick-start" },
      { text: "Connecting MCP clients", link: "/guide/connecting-clients" },
      { text: "CLI reference", link: "/guide/cli-reference" },
      { text: "Capability policy", link: "/guide/capability-policy" },
    ],
  },
];

const serversSidebar = [
  {
    text: "Servers",
    items: [
      { text: "Overview", link: "/servers/" },
      { text: "Filesystem", link: "/servers/filesystem" },
      { text: "Fetch", link: "/servers/fetch" },
      { text: "Memory", link: "/servers/memory" },
      { text: "Shell", link: "/servers/shell" },
      { text: "Skills", link: "/servers/skills" },
      { text: "Agents", link: "/servers/agents" },
    ],
  },
];

const configurationSidebar = [
  {
    text: "Configuration",
    items: [
      { text: "Subagent configuration", link: "/configuration/subagents" },
      { text: "Provider configuration", link: "/configuration/providers" },
      { text: "OpenRouter subagents", link: "/configuration/openrouter" },
      { text: "Child MCP servers", link: "/configuration/child-mcp" },
      { text: "Agent profiles", link: "/configuration/agent-profiles" },
    ],
  },
];

const conceptsSidebar = [
  {
    text: "Concepts",
    items: [
      { text: "Security model", link: "/concepts/security-model" },
      { text: "Limits & bounded behavior", link: "/concepts/limits" },
      { text: "Naming conventions", link: "/concepts/naming" },
      { text: "Workspace layout", link: "/concepts/workspace-layout" },
      {
        text: "Least-privilege example",
        link: "/concepts/least-privilege-example",
      },
    ],
  },
];

export default defineConfig({
  base,
  lang: "en-US",
  title: "tuls",
  description:
    "A compact Rust MCP toolbox for filesystem access, HTTP fetches, persistent memory, local process execution, reusable skills, and provider-backed local subagents.",
  cleanUrls: true,
  lastUpdated: true,
  head: [
    [
      "link",
      { rel: "icon", type: "image/svg+xml", href: `${base}logo-mark.svg` },
    ],
    ["meta", { name: "theme-color", content: "#ea580c" }],
    [
      "meta",
      { property: "og:title", content: "tuls — a compact Rust MCP toolbox" },
    ],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Six focused MCP servers in one binary: filesystem, fetch, memory, shell, skills, and agents. Explicit capabilities, least privilege, bounded I/O.",
      },
    ],
    ["meta", { property: "og:image", content: `${base}logo.svg` }],
  ],
  themeConfig: {
    logo: { src: "/logo-mark.svg", alt: "tuls" },
    siteTitle: "tuls",
    nav,
    sidebar: {
      "/guide/": guideSidebar,
      "/servers/": serversSidebar,
      "/configuration/": configurationSidebar,
      "/concepts/": conceptsSidebar,
      "/troubleshooting": guideSidebar,
      "/development": guideSidebar,
    },
    outline: { level: [2, 3], label: "On this page" },
    docFooter: { prev: "Previous", next: "Next" },
    lastUpdated: {
      text: "Last updated",
      formatOptions: { dateStyle: "full", timeStyle: "short" },
    },
    search: { provider: "local" },
    socialLinks: [
      {
        icon: "github",
        link: "https://github.com/tuls-rs/tuls",
      },
    ],
    footer: {
      message: "MIT licensed · built for MCP 2026-07-28",
      copyright: "Copyright © 2026 the tuls authors",
    },
    returnToTopLabel: "Back to top",
    sidebarMenuLabel: "Menu",
    darkModeSwitchLabel: "Appearance",
    lightModeSwitchTitle: "Switch to light mode",
    darkModeSwitchTitle: "Switch to dark mode",
  },
  markdown: {
    theme: { light: "github-light", dark: "github-dark" },
    image: { lazyLoading: true },
    toc: { level: [2, 3] },
  },
});
