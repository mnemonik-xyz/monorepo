/// <reference types="chrome" />

// Floating action button injected on supported AI-chat domains. Real UI
// implementation lands in T13 (FAB + recall overlay + hotkeys). This
// stub exists so the MV3 manifest's `content_scripts` entries point at
// real bundleable files in T10 without blocking T13.
//
// Per D12 the FAB is the user-gesture surface for capture on supported
// domains; without it, capture only fires via the popup or the
// context-menu (D11). The stub deliberately attaches no listeners.

export {};
