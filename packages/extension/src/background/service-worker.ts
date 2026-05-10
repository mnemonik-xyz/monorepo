/// <reference types="chrome" />

// MV3 service worker entry. T01 scaffolds an empty handler that proves the
// build pipeline emits a service-worker bundle Chrome will accept; real
// dispatch (context-menu wiring, alarm-driven cloud sync, message routing)
// lands in T10 (background dispatch).

chrome.runtime.onInstalled.addListener((details) => {
  console.log("[mnemonik] installed", details.reason);
});

chrome.runtime.onStartup.addListener(() => {
  console.log("[mnemonik] startup");
});

export {};
