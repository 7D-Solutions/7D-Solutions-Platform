import type { Meta, StoryObj } from "@storybook/react";
import { Textarea } from "@7d/ui";

const meta: Meta<typeof Textarea> = {
  title: "Primitives/Textarea",
  component: Textarea,
  tags: ["autodocs"],
  args: {
    placeholder: "Enter text…",
    rows: 4,
  },
};

export default meta;
type Story = StoryObj<typeof Textarea>;

export const Default: Story = {};

export const WithError: Story = {
  args: { error: true },
};

export const Disabled: Story = {
  args: { disabled: true, value: "Disabled content" },
};
