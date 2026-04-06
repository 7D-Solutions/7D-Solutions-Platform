import type { Meta, StoryObj } from "@storybook/react";
import { HelperText } from "@7d/ui";

const meta: Meta<typeof HelperText> = {
  title: "Primitives/HelperText",
  component: HelperText,
  tags: ["autodocs"],
  args: {
    children: "Use 8+ characters including a symbol.",
  },
};

export default meta;
type Story = StoryObj<typeof HelperText>;

export const Hint: Story = {};

export const Error: Story = {
  args: {
    error: true,
    children: "This field is required.",
  },
};
