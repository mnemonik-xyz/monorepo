chrome.runtime.onInstalled.addListener((details) => {
  console.info("[mnemonik] installed", details.reason);
});

chrome.runtime.onMessage.addListener((_msg, _sender, sendResponse) => {
  sendResponse({ ok: true, scaffold: true });
  return false;
});

chrome.commands.onCommand.addListener((command) => {
  console.info("[mnemonik] command", command);
});

export {};
