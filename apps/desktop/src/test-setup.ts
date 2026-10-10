/**
 * Vitest setup, run before every test file.
 *
 * Unmounts whatever the previous test rendered. `@testing-library/react` does this
 * automatically only when Vitest runs with `globals: true`, which this project does not —
 * the tests import `describe`/`it`/`expect` explicitly, which is clearer about where they
 * come from. So the hook is registered here instead.
 *
 * Without it, every `render` in a file accumulates in the same document and a query that
 * should match one element matches several. The symptom is a confusing "found multiple
 * elements" failure in the *second* test of a file while the first passes alone, which is
 * a bad afternoon to debug twice.
 */
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
});
