/**
 * The School logo panel.
 *
 * `evara_core::branding` decides what a usable image is and `tests_school_logo.rs` proves
 * the storage. This file owns what only the screen can get wrong:
 *
 * - a school with no logo shows Evara's, labelled as the default rather than as theirs
 * - a chosen file reaches the backend as base64, with no filename attached
 * - an obviously wrong file is refused before it is read, and the backend's refusal is
 *   shown when one gets past that
 * - replacing and removing do what they say, and removing asks first
 * - nothing about a logo blocks finishing setup
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import type { LogoLimits, School, SchoolLogoView } from "@evara/domain";

const getSchoolLogo = vi.fn<() => Promise<SchoolLogoView | null>>();
const getSchoolLogoLimits = vi.fn<() => Promise<LogoLimits>>();
const setSchoolLogo = vi.fn();
const clearSchoolLogo = vi.fn();

vi.mock("../../ipc/setup", () => ({
  getSchoolLogo: () => getSchoolLogo(),
  getSchoolLogoLimits: () => getSchoolLogoLimits(),
  setSchoolLogo: (id: string, data: string) => setSchoolLogo(id, data),
  clearSchoolLogo: () => clearSchoolLogo(),
  describeSetupError: (error: unknown) =>
    typeof error === "object" && error !== null && "message" in error
      ? String((error as { message: unknown }).message)
      : String(error),
}));

const { SchoolLogoPanel, describeBytes, quickReject } = await import("./SchoolLogoPanel");

const SCHOOL = { id: "school-1", name: "Northgate Secondary" } as School;

const LIMITS: LogoLimits = {
  maxBytes: 2 * 1024 * 1024,
  maxEdge: 2048,
  minEdge: 16,
  mediaTypes: ["image/png", "image/jpeg", "image/webp"],
};

function storedLogo(overrides: Partial<SchoolLogoView> = {}): SchoolLogoView {
  return {
    dataUrl: "data:image/png;base64,SCHOOLLOGO",
    mediaType: "image/png",
    width: 256,
    height: 256,
    byteSize: 4096,
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...overrides,
  } as SchoolLogoView;
}

/** A File whose bytes a jsdom FileReader will happily turn into a data URL. */
function imageFile(name: string, type: string, size = 2048): File {
  const file = new File([new Uint8Array(size)], name, { type });
  // jsdom honours the constructed size, but being explicit keeps the oversize tests
  // readable without allocating megabytes.
  Object.defineProperty(file, "size", { value: size });
  return file;
}

beforeEach(() => {
  vi.clearAllMocks();
  getSchoolLogo.mockResolvedValue(null);
  getSchoolLogoLimits.mockResolvedValue(LIMITS);
  setSchoolLogo.mockResolvedValue(storedLogo());
  clearSchoolLogo.mockResolvedValue(undefined);
});

const fileInput = () => screen.getByLabelText(/logo…$/i) as HTMLInputElement;
const preview = () => screen.getByRole("img") as HTMLImageElement;

async function shown() {
  render(<SchoolLogoPanel school={SCHOOL} />);
  await waitFor(() => expect(getSchoolLogo).toHaveBeenCalled());
  await waitFor(() => expect(screen.queryByText("Loading…")).toBeNull());
}

describe("a school with no logo", () => {
  it("shows Evara's mark, named as the default", async () => {
    await shown();

    // Not an empty box: the fallback is visible, so it is obvious what a school without a
    // logo actually gets.
    expect(preview().getAttribute("alt")).toBe("Evara");
    expect(screen.getByText("Evara default")).toBeTruthy();
  });

  it("offers an upload and nothing to remove", async () => {
    await shown();

    expect(screen.getByText("Upload logo…")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Remove logo" })).toBeNull();
  });

  it("says what is accepted, using the backend's own limits", async () => {
    await shown();

    const hint = screen.getByText(/PNG, JPEG or WebP/);
    expect(hint.textContent).toMatch(/up to 2 MB/);
    expect(hint.textContent).toMatch(/square image with a transparent background/);
    expect(fileInput().getAttribute("accept")).toBe(
      "image/png,image/jpeg,image/webp",
    );
  });

  it("does not block anything: the panel is purely optional", async () => {
    await shown();
    // No required marker, no validation message, nothing that could fail.
    expect(screen.queryByText(/required/i)).toBeNull();
    expect(screen.getByText(/Optional\./)).toBeTruthy();
  });
});

describe("a school with its own logo", () => {
  it("shows it, with its dimensions and size", async () => {
    getSchoolLogo.mockResolvedValue(storedLogo({ width: 300, height: 300, byteSize: 15360 }));
    await shown();

    expect(preview().getAttribute("src")).toBe("data:image/png;base64,SCHOOLLOGO");
    expect(preview().getAttribute("alt")).toBe("Northgate Secondary logo");
    expect(screen.getByText("300×300 · 15 KB")).toBeTruthy();
  });

  it("offers replacing and removing", async () => {
    getSchoolLogo.mockResolvedValue(storedLogo());
    await shown();

    expect(screen.getByText("Replace logo…")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove logo" })).toBeTruthy();
  });

  it("mentions letterboxing when the logo is not square, without refusing it", async () => {
    getSchoolLogo.mockResolvedValue(storedLogo({ width: 512, height: 128 }));
    await shown();

    // Advice, not a rule: a wide wordmark is a real logo.
    expect(screen.getByText(/not square/)).toBeTruthy();
    expect(screen.getByText(/That is fine if it is the logo you have/)).toBeTruthy();
  });

  it("says nothing about shape when the logo is square", async () => {
    getSchoolLogo.mockResolvedValue(storedLogo({ width: 256, height: 256 }));
    await shown();

    expect(screen.queryByText(/not square/)).toBeNull();
  });
});

describe("uploading", () => {
  it("sends the file as base64, with no filename", async () => {
    const user = userEvent.setup();
    await shown();

    await user.upload(fileInput(), imageFile("crest.png", "image/png"));

    await waitFor(() => expect(setSchoolLogo).toHaveBeenCalled());
    const [schoolId, data] = setSchoolLogo.mock.calls.at(-1) as [string, string];
    expect(schoolId).toBe("school-1");
    expect(typeof data).toBe("string");
    expect(data.length).toBeGreaterThan(0);
    // Pure base64: no `data:` prefix, and nothing carrying the name "crest.png". The
    // backend identifies the format from the content.
    expect(data.startsWith("data:")).toBe(false);
    expect(data).not.toMatch(/crest/);
  });

  it("shows the new logo once it is stored", async () => {
    const user = userEvent.setup();
    setSchoolLogo.mockResolvedValue(storedLogo({ dataUrl: "data:image/png;base64,NEW" }));
    await shown();

    await user.upload(fileInput(), imageFile("crest.png", "image/png"));

    await waitFor(() =>
      expect(preview().getAttribute("src")).toBe("data:image/png;base64,NEW"),
    );
    expect(screen.getByText("Replace logo…")).toBeTruthy();
  });

  it("accepts a JPEG and a WebP as readily as a PNG", async () => {
    const user = userEvent.setup();
    await shown();

    for (const [name, type] of [
      ["crest.jpg", "image/jpeg"],
      ["crest.webp", "image/webp"],
    ] as const) {
      setSchoolLogo.mockClear();
      await user.upload(fileInput(), imageFile(name, type));
      await waitFor(() => expect(setSchoolLogo).toHaveBeenCalled());
    }
  });

  it("refuses an oversized file without sending it", async () => {
    const user = userEvent.setup();
    await shown();

    await user.upload(
      fileInput(),
      imageFile("huge.png", "image/png", LIMITS.maxBytes + 1),
    );

    expect(await screen.findByText(/The limit is 2 MB/)).toBeTruthy();
    // The point of checking early: a 40 MB photograph never gets read into memory or
    // base64'd across the IPC boundary.
    expect(setSchoolLogo).not.toHaveBeenCalled();
  });

  it("refuses an obviously unsupported type without sending it", async () => {
    // `applyAccept: false` models a user who got past the dialogue's own filter -- an
    // "All files" picker, or a drag-and-drop. The `accept` attribute is a convenience,
    // never a guarantee, which is why the panel checks as well.
    const user = userEvent.setup({ applyAccept: false });
    await shown();

    await user.upload(fileInput(), imageFile("notes.pdf", "application/pdf"));

    expect(
      await screen.findByText(/not a PNG, JPEG or WebP image/),
    ).toBeTruthy();
    expect(setSchoolLogo).not.toHaveBeenCalled();
  });

  it("shows the backend's refusal when a file gets past the early check", async () => {
    // A renamed file: the browser calls it a PNG, the bytes say otherwise, and only the
    // backend can tell. Its message is what the user sees.
    const user = userEvent.setup();
    setSchoolLogo.mockRejectedValue({
      kind: "invalid",
      message: "that file is not a PNG, JPEG or WebP image",
    });
    await shown();

    await user.upload(fileInput(), imageFile("actually-a-zip.png", "image/png"));

    expect(
      await screen.findByText("that file is not a PNG, JPEG or WebP image"),
    ).toBeTruthy();
  });

  it("keeps showing the existing logo when a replacement is refused", async () => {
    const user = userEvent.setup();
    getSchoolLogo.mockResolvedValue(storedLogo({ dataUrl: "data:image/png;base64,OLD" }));
    setSchoolLogo.mockRejectedValue({ kind: "invalid", message: "that image is damaged" });
    await shown();

    await user.upload(fileInput(), imageFile("broken.png", "image/png"));

    expect(await screen.findByText("that image is damaged")).toBeTruthy();
    // A failed replacement must not lose what was already there.
    expect(preview().getAttribute("src")).toBe("data:image/png;base64,OLD");
  });
});

describe("removing", () => {
  it("asks first", async () => {
    const user = userEvent.setup();
    getSchoolLogo.mockResolvedValue(storedLogo());
    await shown();

    await user.click(screen.getByRole("button", { name: "Remove logo" }));

    expect(await screen.findByText(/Remove this school's logo\?/)).toBeTruthy();
    expect(clearSchoolLogo).not.toHaveBeenCalled();
  });

  it("does nothing if the confirmation is cancelled", async () => {
    const user = userEvent.setup();
    getSchoolLogo.mockResolvedValue(storedLogo());
    await shown();

    await user.click(screen.getByRole("button", { name: "Remove logo" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(clearSchoolLogo).not.toHaveBeenCalled();
    expect(preview().getAttribute("src")).toBe("data:image/png;base64,SCHOOLLOGO");
  });

  it("restores the Evara default once confirmed", async () => {
    const user = userEvent.setup();
    getSchoolLogo.mockResolvedValue(storedLogo());
    await shown();

    await user.click(screen.getByRole("button", { name: "Remove logo" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove logo" }));

    await waitFor(() => expect(clearSchoolLogo).toHaveBeenCalled());
    await waitFor(() => expect(preview().getAttribute("alt")).toBe("Evara"));
    expect(screen.getByText("Evara default")).toBeTruthy();
    expect(screen.getByText("Upload logo…")).toBeTruthy();
  });
});

describe("when the logo cannot be read at all", () => {
  it("falls back to Evara's mark and says what went wrong", async () => {
    getSchoolLogo.mockRejectedValue({ kind: "internal", message: "the project is unreadable" });
    render(<SchoolLogoPanel school={SCHOOL} />);

    // The fallback is a correct answer, so a read failure is reported without the panel
    // refusing to render.
    expect(await screen.findByText("the project is unreadable")).toBeTruthy();
    await waitFor(() => expect(preview().getAttribute("alt")).toBe("Evara"));
  });

  it("still works when the limits cannot be fetched", async () => {
    // The limits only drive hint text and the early check; the backend enforces
    // everything regardless, so this must not become an error banner.
    getSchoolLogoLimits.mockRejectedValue(new Error("nope"));
    const user = userEvent.setup();
    await shown();

    expect(screen.queryByText("nope")).toBeNull();
    await user.upload(fileInput(), imageFile("crest.png", "image/png"));
    await waitFor(() => expect(setSchoolLogo).toHaveBeenCalled());
  });
});

describe("the helpers", () => {
  it("describes byte counts the way a person reads them", () => {
    expect(describeBytes(512)).toBe("512 bytes");
    expect(describeBytes(15360)).toBe("15 KB");
    expect(describeBytes(1024 * 1024)).toBe("1 MB");
    expect(describeBytes(2 * 1024 * 1024)).toBe("2 MB");
    // A real decimal is kept; only a trailing ".0" is dropped.
    expect(describeBytes(1_887_437)).toBe("1.8 MB");
    expect(describeBytes(15 * 1024 * 1024)).toBe("15 MB");
  });

  it("lets anything through when the limits are unknown", () => {
    // Better to send and let the backend decide than to refuse on a guess.
    expect(quickReject(imageFile("x.png", "image/png"), null)).toBeNull();
  });

  it("refuses an empty file", () => {
    expect(quickReject(imageFile("x.png", "image/png", 0), LIMITS)).toMatch(/empty/);
  });

  it("lets a file with no declared type through to the backend", () => {
    // Some systems report no type at all. Guessing would be exactly the mistake the
    // backend's content sniffing exists to avoid.
    expect(quickReject(imageFile("x", "", 1024), LIMITS)).toBeNull();
  });
});
