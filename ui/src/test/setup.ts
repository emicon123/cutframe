import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import { useToastStore } from "@/stores/toastStore";

afterEach(() => {
  cleanup();
  clearMocks();
  useToastStore.setState({ toasts: [] });
});
