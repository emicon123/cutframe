import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { type AboutInfo_Serialize, mockCommands } from "@/bindings";
import { t } from "@/i18n";
import { useAppStore } from "@/stores/appStore";
import { useUiStore } from "@/stores/uiStore";

import { AboutDialog } from "./AboutDialog";
import { LICENSE_PAGE_SIZE, type ThirdPartyLicenses, type ThirdPartyPackage } from "./licenses";

const info: AboutInfo_Serialize = {
  version: "0.1.0",
  buildProfile: "debug",
  commit: "1234567890ab",
  ffmpegVersion: "n7.1.5",
  ffmpegLicense: "LGPL version 3 or later",
  logDir: "C:\\Users\\me\\AppData\\Local\\app.cutframe.editor\\logs",
};

const pkg = (i: number): ThirdPartyPackage => ({
  ecosystem: i % 2 ? "npm" : "cargo",
  name: `pkg-${String(i).padStart(3, "0")}`,
  version: "1.0.0",
  license: i === 7 ? "Zlib" : "MIT",
  repository: `https://example.test/pkg-${i}`,
  homepage: null,
  authors: [],
  licenseTextIds: ["mit"],
});

const licenses: ThirdPartyLicenses = {
  schemaVersion: 1,
  ffmpeg: {
    name: "FFmpeg",
    version: "7.1.5",
    branch: "7.1",
    variant: "win64-lgpl-shared",
    buildTag: "autobuild",
    license: "LGPL-3.0-or-later",
    sourceCode: "https://example.test/ffmpeg-source",
    buildScripts: "https://example.test/ffmpeg-builds",
    binaryUrl: "https://example.test/ffmpeg.zip",
    sha256: "00",
    dlls: ["avcodec-61.dll"],
    notice: "Cutframe uses the FFmpeg libraries",
    sourceOffer: "The complete corresponding source code is available",
    licenseTextIds: ["lgpl"],
  },
  packages: Array.from({ length: LICENSE_PAGE_SIZE + 10 }, (_, i) => pkg(i)),
  licenseTexts: {
    mit: { name: "LICENSE-MIT", text: "Permission is hereby granted" },
    lgpl: { name: "LGPL-3.0-or-later", text: "GNU LESSER GENERAL PUBLIC LICENSE" },
  },
};

afterEach(() => {
  useUiStore.setState({ aboutOpen: false });
  useAppStore.setState({ previewOccluders: 0 });
});

/** Radix tabs switch on mouse down, not on click. */
const openTab = async (name: string) => fireEvent.mouseDown(await screen.findByRole("tab", { name }));

function setup(
  about: AboutInfo_Serialize = info,
  load = () => Promise.resolve<ThirdPartyLicenses | null>(licenses),
) {
  mockCommands({ getAboutInfo: () => about });
  useUiStore.setState({ aboutOpen: true });
  render(<AboutDialog loadLicenses={load} />);
}

describe("AboutDialog", () => {
  it("shows version, build, commit, FFmpeg and the log folder and hides the preview", async () => {
    setup();
    const dialog = await screen.findByRole("dialog", { name: t.about.title });
    expect(await within(dialog).findByText(info.version)).toBeTruthy();
    expect(within(dialog).getByText(t.about.buildProfiles.debug)).toBeTruthy();
    expect(within(dialog).getByText(info.commit ?? "")).toBeTruthy();
    expect(within(dialog).getByText(info.ffmpegVersion)).toBeTruthy();
    expect(within(dialog).getByText(info.ffmpegLicense)).toBeTruthy();
    expect(within(dialog).getByText(info.logDir ?? "")).toBeTruthy();
    expect(useAppStore.getState().previewOccluders).toBe(1);
  });

  it("leaves out an unknown commit and says when there is no log folder", async () => {
    setup({ ...info, commit: null, logDir: null, buildProfile: "release" });
    expect(await screen.findByText(t.about.buildProfiles.release)).toBeTruthy();
    expect(screen.queryByText(t.about.commit)).toBeNull();
    expect(screen.getByText(t.about.noLogFolder)).toBeTruthy();
    expect(screen.queryByRole("button", { name: t.about.openLogs })).toBeNull();
  });

  it("lists the FFmpeg notice and a page of packages, with search and licence texts on demand", async () => {
    setup();
    await openTab(t.about.tabs.licenses);
    expect(await screen.findByText(licenses.ffmpeg.notice)).toBeTruthy();
    expect(screen.getByText(licenses.ffmpeg.sourceOffer)).toBeTruthy();
    expect(screen.getByText(licenses.ffmpeg.sourceCode)).toBeTruthy();
    expect(screen.getByText(licenses.ffmpeg.buildScripts)).toBeTruthy();
    expect(screen.getByText("GNU LESSER GENERAL PUBLIC LICENSE")).toBeTruthy();

    const rows = () => screen.getAllByRole("button", { name: /^Show licence text of / });
    expect(rows()).toHaveLength(LICENSE_PAGE_SIZE);
    fireEvent.click(screen.getByRole("button", { name: t.about.licenses.showMore(10) }));
    expect(rows()).toHaveLength(LICENSE_PAGE_SIZE + 10);

    // No package text is rendered until its row is expanded.
    expect(screen.queryByText("Permission is hereby granted")).toBeNull();
    fireEvent.change(screen.getByRole("searchbox", { name: t.about.licenses.search }), {
      target: { value: "zlib" },
    });
    await waitFor(() => expect(rows()).toHaveLength(1));
    const row = screen.getByRole("button", { name: t.about.licenses.showLicense("pkg-007") });
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Permission is hereby granted")).toBeTruthy();
    expect(screen.getByText("https://example.test/pkg-7")).toBeTruthy();
  });

  it("says so when the notices were not generated", async () => {
    setup(info, () => Promise.resolve(null));
    await openTab(t.about.tabs.licenses);
    expect(await screen.findByText(t.about.licenses.missing)).toBeTruthy();
  });
});
