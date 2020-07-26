// Chrome doesn't need cloneInto, but Firefox does.
const cloneIntoPolyfill = typeof cloneInto !== 'undefined' ? cloneInto : ((m, _) => m);

const port = chrome.runtime.connect();
document.addEventListener("voicesurf.browser", (message) => {
    port.postMessage(message.detail);
});

port.onMessage.addListener((message) => {
    document.dispatchEvent(
        new CustomEvent(
            "voicesurf.native",
            cloneIntoPolyfill(message, document.defaultView),
        ),
    );
});

const watcher = function () {
    /**
     * Tracks given elements using weak references, creates the
     * bidirectional map between these elements and a unique
     * identifier.
     */
    class ElementTracker {
        constructor() {
            this.currentId = 0;
            this.elToId = new WeakMap();
            this.idToRef = new Map();
            this.finalizationGroup = new FinalizationRegistry(
                ElementTracker.cleanup,
            );
        }

        cleanup({ idToRef, id }) {
            idToRef.delete(id);
        }

        /**
         * Track el, and mint a unique ID which can be used in
         * `getElById` for lookups.
         */
        track(el) {
            const id = this.currentId++;
            const ref = new WeakRef(el);
            this.elToId.set(el, id);
            this.idToRef.set(id, ref);
            this.finalizationGroup.register(
                el,
                {
                    idToRef: this.idToRef,
                    id,
                },
                ref,
            );
            return id;
        }

        /**
         * Stop tracking el and return its ID. Note that because this
         * class uses weak references, it should suffice to let el get
         * garbage collected naturally. However, this is available for
         * manual collection.
         *
         * TODO(kvakil): do we want this? It feels like a mix of manual
         * memory management and WeakRefs for garbage collection. I'm
         * not sure which gives better performance in practice.
         */
        untrack(el) {
            const id = this.elToId.get(el);
            if (!id) {
                return undefined;
            }
            const ref = this.elToId.get(ref);
            this.idToRef.delete(id);
            this.elToId.delete(el);
            this.finalizationGroup.unregister(ref);
            return id;
        }

        getElById(id) {
            return this.idToRef.get(id)?.deref();
        }
    }

    const et = new ElementTracker();

    /**
     * determines if the given element is visible in the current viewport
     */
    function isVisible(el) {
        const elRect = el.getBoundingClientRect();
        const viewHeight = Math.max(
            document.documentElement.clientHeight,
            window.innerHeight,
        );
        return elRect.bottom >= 0 && elRect.top - viewHeight < 0;
    }

    document.addEventListener("voicesurf.native", (message) => {
        for (const elId of message.detail) {
            const el = et.getElById(elId);
            if (el && isVisible(el)) {
                el.click();
                break;
            }
        }
    });

    /**
     * Selectors for "clickable" elements.
     * If you update this, be sure to change shouldTrack below.
     */
    const selectors = [
        "a",
        "[role=button]",
        "input[type=button]",
        "input[type=submit]",
        "input[type=reset]",
        "input[type=image]",
    ];
    /**
     * Determines if the given element is clickable, so that we should
     * track it for updates.
     */
    function shouldTrack(el) {
        // Not using el.matches, because it's probably faster to use
        // tagName / manual getters.
        if (el instanceof Element) {
            const tagName = el.tagName;
            if (tagName === "A" || tagName === "LABEL") {
                return true;
            } else if (tagName === "INPUT") {
                const type = el.getAttribute("type");
                if (
                    type === "button" ||
                    type === "submit" ||
                    type === "reset" ||
                    type === "image"
                ) {
                    return true;
                }
            } else if (el.getAttribute("role") === "button") {
                return true;
            }
        }
        return false;
    }

    /**
     * Watches any given element for text updates.
     */
    const textUpdateObserver = new MutationObserver((mutations) => {
        let updated = [];
        for (let i = 0; i < mutations.length; i++) {
            const mutation = mutations[i];
            const target = mutation.target;
            const idOrUndefined = et.elToId.get(el);
            if (idOrUndefined) {
                updated.push([idOrUndefined, el.textContent]);
            }
        }
        document.dispatchEvent(
            new CustomEvent("voicesurf.browser", {
                detail: { UpdateIndex: { updated, removed: [] } },
            }),
        );
    });

    function watch(el) {
        textUpdateObserver.observe(el, {
            subtree: true,
            childList: true,
            characterData: true,
        });
        const id = et.track(el);
        return id;
    }

    function unwatch(el) {
        const id = et.untrack(el);
        if (id) {
            textUpdateObserver.disconnect(el);
        }
        return id;
    }

    /**
     * Watch for new clickable elements, or old clickable elements being
     * removed.
     */
    new MutationObserver((mutations) => {
        // Note that this function runs VERY often, and should be
        // very optimized.
        // TODO(kvakil): profile this.
        const updateIndex = { updated: [], removed: [] };
        for (let i = 0; i < mutations.length; i++) {
            const mutation = mutations[i];
            // TODO(kvakil): is it necessary to process removedNodes
            // first? I am doing so under the assumption that a node can
            // be added and removed, but I haven't seen that in
            // practice.
            const removedNodes = mutation.removedNodes;
            for (let j = 0; j < removedNodes.length; j++) {
                const el = removedNodes[j];
                const id = unwatch(el);
                if (id) {
                    updateIndex.removed.push(id);
                }
            }
            const addedNodes = mutation.addedNodes;
            for (let j = 0; j < addedNodes.length; j++) {
                const el = addedNodes[j];
                if (shouldTrack(el)) {
                    const id = watch(el);
                    // We use textContent here. Using innerText would be
                    // better, but it may require a reflow. This
                    // function is called pretty often, so I think we'd
                    // prefer using textContent.
                    //
                    // TODO(kvakil): other accessibility elements and
                    // what attributes are useful besides textContent?
                    updatedIndex.updated.push([id, el.textContent]);
                }
            }
        }

        if (updateIndex.updated.length > 0 || updateIndex.removed.length > 0) {
            document.dispatchEvent(
                new CustomEvent("voicesurf.browser", {
                    detail: { UpdateIndex: updateIndex },
                }),
            );
        }
    }).observe(document.body, { childList: true, subtree: true });

    // Compute an initial index.
    const updateIndex = { updated: [], removed: [] };
    selectors
        .flatMap((selector) => Array.from(document.querySelectorAll(selector)))
        .forEach((el) => {
            const id = watch(el);
            updateIndex.updated.push([id, el.textContent]);
        });
    document.dispatchEvent(
        new CustomEvent("voicesurf.browser", {
            detail: { UpdateIndex: updateIndex },
        }),
    );
};

// For some reason, WeakRef doesn't work properly in Firefox when a
// content script gets elements from the page's DOM. Inject the script
// into the document instead. (This forces a layer of indirection by
// dispatchEvent, but that's not a big deal.)
const script = document.createElement("script");
script.textContent = `(${watcher})()`;
(document.head || document.documentElement).appendChild(script);
script.remove();
