let nativePort;
let contentPortById = new Map();
chrome.runtime.onConnect.addListener((contentPort) => {
    const tabId = contentPort.sender.tab.id;
    contentPortById.set(tabId, contentPort);
    contentPort.onMessage.addListener((data) => {
        if (data.UpdateIndex) {
            data.UpdateIndex.tabId = tabId;
        } else if (data.FocusTab) {
            data.FocusTab.tabId = tabId;
        }
        nativePort.postMessage(data);
    });
    contentPort.onDisconnect.addListener((contentPort) =>
        contentPortById.delete(contentPort.sender.tab.id),
    );
});

nativePort = chrome.runtime.connectNative("voicesurf");
nativePort.onMessage.addListener((response) => {
    const tabId = response.tabId;
    contentPortById.get(tabId)?.postMessage({ detail: response.best });
});
