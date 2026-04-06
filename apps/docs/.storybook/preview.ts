import type { Preview } from "@storybook/react";
import "../src/index.css";

const preview: Preview = {
  parameters: {
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /date$/i,
      },
    },
    layout: "centered",
  },
  decorators: [
    (Story) => {
      if (typeof document !== "undefined") {
        document.documentElement.setAttribute("data-brand", "trashtech");
      }
      return Story();
    },
  ],
};

export default preview;
