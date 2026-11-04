import { useEffect, useState, type MutableRef } from "preact/hooks";

/** Tracks an element's rendered height, updating as its layout changes. */
export function useElementHeight(elementRef: MutableRef<HTMLElement | null>): number {
  const [height, setHeight] = useState(0);
  useEffect(() => {
    const element = elementRef.current;
    if (!element) {
      return;
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) {
        setHeight(entry.contentRect.height);
      }
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [elementRef]);
  return height;
}
