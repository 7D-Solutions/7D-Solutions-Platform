// ============================================================
// Tab management store — port from Fireproof tabStore
// Persisted to localStorage. All state is tab-scoped via activeTabId.
// ============================================================
'use client';
import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { useShallow } from 'zustand/shallow';

export interface TabData {
  id: string;
  title: string;
  route: string;
  icon?: string;
  isDirty?: boolean;
  isPreview?: boolean;
  hasOpenModal?: boolean;
  data?: Record<string, unknown>;
  closeable?: boolean;
}

interface TabState {
  tabs: TabData[];
  activeTabId: string;
  splitView: {
    enabled: boolean;
    leftPaneTabId: string | null;
    rightPaneTabId: string | null;
    dividerPosition: number;
  };

  openTab: (tab: Omit<TabData, 'id'> & { id?: string }) => string;
  closeTab: (tabId: string) => void;
  setActiveTab: (tabId: string) => void;
  updateTab: (tabId: string, updates: Partial<TabData>) => void;
  reorderTabs: (tabIds: string[]) => void;
  promotePreviewTab: (tabId: string) => void;
  openPreviewTab: (tab: Omit<TabData, 'id' | 'isPreview'> & { id?: string }) => string;
  setTabDirty: (tabId: string, dirty: boolean) => void;

  enableSplitView: (leftTabId: string, rightTabId: string) => void;
  disableSplitView: () => void;
  setDividerPosition: (position: number) => void;

  closeAllTabs: () => void;
  closeOtherTabs: (tabId: string) => void;
  getTab: (tabId: string) => TabData | undefined;
  findTabByRoute: (route: string) => TabData | undefined;
}

const generateTabId = () => `tab_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

const HOME_TAB: TabData = {
  id: 'home',
  title: 'Tenants',
  route: '/app/tenants',
  closeable: false,
  isPreview: false,
};

export const useTabStore = create<TabState>()(
  persist(
    (set, get) => ({
      tabs: [HOME_TAB],
      activeTabId: 'home',
      splitView: {
        enabled: false,
        leftPaneTabId: null,
        rightPaneTabId: null,
        dividerPosition: 50,
      },

      openTab: (tabData) => {
        const { tabs, activeTabId } = get();

        if (tabData.id) {
          const existing = tabs.find((t) => t.id === tabData.id);
          if (existing) {
            set({ activeTabId: tabData.id });
            return tabData.id;
          }
        }

        const existingByRoute = tabs.find((t) => t.route === tabData.route);
        if (existingByRoute) {
          if (existingByRoute.isPreview && !tabData.isPreview) {
            get().updateTab(existingByRoute.id, { isPreview: false });
          }
          set({ activeTabId: existingByRoute.id });
          return existingByRoute.id;
        }

        const newTab: TabData = {
          ...tabData,
          id: tabData.id || generateTabId(),
          closeable: tabData.closeable !== false,
          isPreview: tabData.isPreview !== false,
        };

        if (newTab.isPreview) {
          const existingPreview = tabs.find((t) => t.isPreview);
          if (existingPreview) {
            const newTabs = tabs.map((t) => (t.id === existingPreview.id ? newTab : t));
            set({ tabs: newTabs, activeTabId: newTab.id });
            return newTab.id;
          }
        }

        const activeIndex = tabs.findIndex((t) => t.id === activeTabId);
        const newTabs = [
          ...tabs.slice(0, activeIndex + 1),
          newTab,
          ...tabs.slice(activeIndex + 1),
        ];
        set({ tabs: newTabs, activeTabId: newTab.id });
        return newTab.id;
      },

      closeTab: (tabId) => {
        const { tabs, activeTabId, splitView } = get();
        const tab = tabs.find((t) => t.id === tabId);
        if (!tab || tab.closeable === false) return;

        let newSplitView = { ...splitView };
        if (
          splitView.enabled &&
          (splitView.leftPaneTabId === tabId || splitView.rightPaneTabId === tabId)
        ) {
          newSplitView = { enabled: false, leftPaneTabId: null, rightPaneTabId: null, dividerPosition: 50 };
        }

        if (activeTabId === tabId) {
          const tabIndex = tabs.findIndex((t) => t.id === tabId);
          let newActiveTabId = tabIndex > 0 ? tabs[tabIndex - 1].id : tabs.length > 1 ? tabs[1].id : 'home';
          set({ activeTabId: newActiveTabId });
          setTimeout(() => {
            const current = get().tabs;
            set({ tabs: current.filter((t) => t.id !== tabId), splitView: newSplitView });
          }, 0);
        } else {
          set({ tabs: tabs.filter((t) => t.id !== tabId), splitView: newSplitView });
        }
      },

      setActiveTab: (tabId) => {
        if (get().tabs.find((t) => t.id === tabId)) {
          set({ activeTabId: tabId });
        }
      },

      updateTab: (tabId, updates) => {
        set((state) => ({
          tabs: state.tabs.map((t) => (t.id === tabId ? { ...t, ...updates } : t)),
        }));
      },

      reorderTabs: (tabIds) => {
        const { tabs } = get();
        const reordered = tabIds
          .map((id) => tabs.find((t) => t.id === id))
          .filter((t): t is TabData => t !== undefined);
        set({ tabs: reordered });
      },

      promotePreviewTab: (tabId) => get().updateTab(tabId, { isPreview: false }),

      openPreviewTab: (tabData) => get().openTab({ ...tabData, isPreview: true }),

      setTabDirty: (tabId, dirty) => get().updateTab(tabId, { isDirty: dirty }),

      enableSplitView: (leftTabId, rightTabId) => {
        const { tabs } = get();
        if (tabs.find((t) => t.id === leftTabId) && tabs.find((t) => t.id === rightTabId)) {
          set({ splitView: { enabled: true, leftPaneTabId: leftTabId, rightPaneTabId: rightTabId, dividerPosition: 50 } });
        }
      },

      disableSplitView: () => {
        set({ splitView: { enabled: false, leftPaneTabId: null, rightPaneTabId: null, dividerPosition: 50 } });
      },

      setDividerPosition: (position) => {
        set((state) => ({
          splitView: { ...state.splitView, dividerPosition: Math.max(20, Math.min(80, position)) },
        }));
      },

      closeAllTabs: () => {
        set({ tabs: [HOME_TAB], activeTabId: 'home', splitView: { enabled: false, leftPaneTabId: null, rightPaneTabId: null, dividerPosition: 50 } });
      },

      closeOtherTabs: (tabId) => {
        const { tabs } = get();
        const keepTab = tabs.find((t) => t.id === tabId);
        if (!keepTab) return;
        set({ tabs: tabs.filter((t) => t.id === 'home' || t.id === tabId), activeTabId: tabId, splitView: { enabled: false, leftPaneTabId: null, rightPaneTabId: null, dividerPosition: 50 } });
      },

      getTab: (tabId) => get().tabs.find((t) => t.id === tabId),
      findTabByRoute: (route) => get().tabs.find((t) => t.route === route),
    }),
    {
      name: 'tcp-tab-storage',
      partialize: (state) => ({ tabs: state.tabs, activeTabId: state.activeTabId }),
    }
  )
);

export const useTabs = () => useTabStore((s) => s.tabs);
export const useActiveTabId = () => useTabStore((s) => s.activeTabId);
export const useSplitView = () => useTabStore((s) => s.splitView);
export const useTabActions = () =>
  useTabStore(
    useShallow((s) => ({
      openTab: s.openTab,
      closeTab: s.closeTab,
      setActiveTab: s.setActiveTab,
      updateTab: s.updateTab,
      reorderTabs: s.reorderTabs,
      promotePreviewTab: s.promotePreviewTab,
      openPreviewTab: s.openPreviewTab,
      setTabDirty: s.setTabDirty,
      enableSplitView: s.enableSplitView,
      disableSplitView: s.disableSplitView,
      setDividerPosition: s.setDividerPosition,
      closeAllTabs: s.closeAllTabs,
      closeOtherTabs: s.closeOtherTabs,
      getTab: s.getTab,
      findTabByRoute: s.findTabByRoute,
    }))
  );
