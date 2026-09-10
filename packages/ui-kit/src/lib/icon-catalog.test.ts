import { existsSync, readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  FALLBACK_ICON_REFERENCE,
  ICON_SECTIONS,
  iconCatalogEntry,
  resolveIconReference,
  searchIconSections,
} from "./icon-catalog";
import { BRAND_ICON_LOADERS, SIMPLE_ICON_LOADERS } from "./icon-loaders";
import { TABLER_COMPONENTS } from "./tabler-icon-components";

const iconsDir = (name: string) =>
  fileURLToPath(new URL(`../assets/icons/${name}/`, import.meta.url));

const names = (source: string) =>
  ICON_SECTIONS.find((section) => section.source === source)!.entries.map((entry) => entry.name);

describe("icon catalog", () => {
  it("has no duplicate references", () => {
    const all = ICON_SECTIONS.flatMap((section) => section.entries.map((entry) => entry.reference));
    expect(new Set(all).size).toBe(all.length);
  });

  it("matches every loader and component map exactly", () => {
    expect(names("brand").sort()).toEqual(Object.keys(BRAND_ICON_LOADERS).sort());
    expect(names("simple-icons").sort()).toEqual(Object.keys(SIMPLE_ICON_LOADERS).sort());
    expect(names("tabler").sort()).toEqual(Object.keys(TABLER_COMPONENTS).sort());
    for (const component of Object.values(TABLER_COMPONENTS)) expect(component).toBeDefined();
  });

  it("vendors exactly the catalogued SVGs, each a plain SVG", () => {
    for (const [source, dir] of [
      ["brand", "brand"],
      ["simple-icons", "simple-icons"],
    ] as const) {
      const files = readdirSync(iconsDir(dir))
        .filter((file) => file.endsWith(".svg"))
        .map((file) => file.slice(0, -4));
      expect(files.sort()).toEqual(names(source).sort());
      for (const file of files) {
        const markup = readFileSync(`${iconsDir(dir)}${file}.svg`, "utf8").trim();
        expect(markup.startsWith("<svg")).toBe(true);
        expect(/<\s*script/i.test(markup)).toBe(false);
      }
      expect(existsSync(`${iconsDir(dir)}LICENSE${source === "brand" ? "" : ".md"}`)).toBe(true);
    }
  });

  it("does not offer a Simple Icons mark for a brand already vendored", () => {
    const vendored = new Set(names("brand"));
    for (const name of names("simple-icons")) expect(vendored.has(name)).toBe(false);
    for (const duplicate of ["docker", "pihole", "truenas", "ubiquiti"]) {
      expect(names("simple-icons")).not.toContain(duplicate);
    }
  });
});

describe("resolveIconReference", () => {
  it("takes the first candidate the build has", () => {
    expect(resolveIconReference(["tabler:server-2", "brand:docker"]).reference).toBe(
      "tabler:server-2",
    );
    expect(resolveIconReference(["simple-icons:jellyfin"]).source).toBe("simple-icons");
  });

  it("skips unknown prefixes, missing keys, and unprefixed strings", () => {
    for (const bad of ["tabler:not-a-real-icon", "simple-icons:docker", "material:home", "server"]) {
      expect(iconCatalogEntry(bad)).toBeUndefined();
      expect(resolveIconReference([bad, "brand:docker"]).reference).toBe("brand:docker");
    }
  });

  it("falls back to the generic server", () => {
    expect(resolveIconReference([null, undefined, "nope:nothing"]).reference).toBe(
      FALLBACK_ICON_REFERENCE,
    );
  });
});

describe("searchIconSections", () => {
  it("returns every section for an empty query", () => {
    expect(searchIconSections("  ")).toHaveLength(ICON_SECTIONS.length);
  });

  it("filters all four sources at once and drops empty sections", () => {
    const sections = searchIconSections("server");
    expect(sections.map((section) => section.source)).toEqual(["lucide", "tabler", "simple-icons"]);
    expect(sections.flatMap((section) => section.entries.map((entry) => entry.reference))).toEqual(
      expect.arrayContaining(["lucide:server", "tabler:server-2", "simple-icons:jellyfin"]),
    );
  });

  it("requires every term to match", () => {
    const matches = searchIconSections("media streaming").flatMap((section) => section.entries);
    expect(matches.map((entry) => entry.name)).toEqual(["jellyfin", "plex", "emby"]);
    expect(searchIconSections("zzzz-no-such-icon")).toEqual([]);
  });
});
