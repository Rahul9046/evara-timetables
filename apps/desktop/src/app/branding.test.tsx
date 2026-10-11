/**
 * The fallback rule, and the brand asset it falls back to.
 *
 * `resolveLogo` is one short function, which is exactly why it is worth pinning: every
 * view that shows a logo goes through it, including views that do not exist yet. If it
 * ever decides that an empty string is a logo, four screens render a broken image at once.
 */
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import type { SchoolLogoView } from "@evara/domain";

vi.mock("../ipc/setup", () => ({
  getSchoolLogo: vi.fn(() => Promise.resolve(null)),
}));

const { EVARA_LOGO, LogoMark, evaraBrand, resolveLogo } = await import("./branding");

function logo(overrides: Partial<SchoolLogoView> = {}): SchoolLogoView {
  return {
    dataUrl: "data:image/png;base64,AAAA",
    mediaType: "image/png",
    width: 256,
    height: 256,
    byteSize: 4096,
    updatedAt: "2026-01-01T00:00:00.000Z",
    ...overrides,
  } as SchoolLogoView;
}

describe("a school with its own logo", () => {
  it("shows it, and says whose it is", () => {
    const resolved = resolveLogo(logo(), "Northgate Secondary");

    expect(resolved.src).toBe("data:image/png;base64,AAAA");
    expect(resolved.isSchoolLogo).toBe(true);
    expect(resolved.alt).toBe("Northgate Secondary logo");
  });

  it("is named generically when the school's name is not to hand", () => {
    expect(resolveLogo(logo()).alt).toBe("School logo");
    expect(resolveLogo(logo(), "   ").alt).toBe("School logo");
    expect(resolveLogo(logo(), null).alt).toBe("School logo");
  });
});

describe("a school without one", () => {
  it("falls back to Evara's mark", () => {
    const resolved = resolveLogo(null, "Northgate Secondary");

    expect(resolved.src).toBe(EVARA_LOGO);
    expect(resolved.isSchoolLogo).toBe(false);
    // Never announced as "Northgate Secondary logo": it is not their logo.
    expect(resolved.alt).toBe("Evara");
  });

  it("treats a logo that has not loaded yet the same way", () => {
    // `undefined` is "still reading", and resolving it to the Evara mark is what stops a
    // view flickering between two different images on load.
    expect(resolveLogo(undefined).src).toBe(EVARA_LOGO);
    expect(resolveLogo(undefined).isSchoolLogo).toBe(false);
  });

  it("treats unusable logo data as absent rather than rendering it", () => {
    // A row written by a future build, or a damaged one. A recognisable Evara mark beats
    // a torn-paper icon, and the alternative is a broken image with no explanation.
    for (const broken of [
      logo({ dataUrl: "" }),
      logo({ dataUrl: "   " }),
      logo({ dataUrl: undefined as unknown as string }),
    ]) {
      const resolved = resolveLogo(broken);
      expect(resolved.src).toBe(EVARA_LOGO);
      expect(resolved.isSchoolLogo).toBe(false);
    }
  });
});

describe("the application's own branding", () => {
  it("is always Evara's, with no school involved", () => {
    const brand = evaraBrand();
    expect(brand.src).toBe(EVARA_LOGO);
    expect(brand.isSchoolLogo).toBe(false);
    expect(brand.alt).toBe("Evara");
  });

  it("is the same asset the fallback uses", () => {
    // Deliberate: a school with no logo is shown the application's mark, not a blank
    // space, so these must be one asset rather than two that could drift.
    expect(evaraBrand().src).toBe(resolveLogo(null).src);
  });
});

describe("LogoMark", () => {
  it("renders an image with the resolved source and name", () => {
    render(<LogoMark logo={resolveLogo(logo(), "Northgate")} size={48} />);

    const image = screen.getByRole("img", { name: "Northgate logo" }) as HTMLImageElement;
    expect(image.getAttribute("src")).toBe("data:image/png;base64,AAAA");
    expect(image.getAttribute("width")).toBe("48");
    expect(image.getAttribute("height")).toBe("48");
  });

  it("always has an accessible name, including on the fallback", () => {
    render(<LogoMark logo={resolveLogo(null)} />);
    expect(screen.getByRole("img", { name: "Evara" })).toBeTruthy();
  });
});
