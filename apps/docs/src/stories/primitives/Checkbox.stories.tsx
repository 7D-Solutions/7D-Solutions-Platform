import type { Meta, StoryObj } from "@storybook/react";
import { Checkbox } from "@7d/ui";

const meta: Meta<typeof Checkbox> = {
  title: "Primitives/Checkbox",
  component: Checkbox,
  tags: ["autodocs"],
  args: {
    label: "Accept terms",
  },
};

export default meta;
type Story = StoryObj<typeof Checkbox>;

export const Default: Story = {};

export const Checked: Story = {
  args: { defaultChecked: true },
};

export const WithError: Story = {
  args: {
    label: "Required checkbox",
    error: true,
  },
};

export const Disabled: Story = {
  args: { disabled: true, label: "Disabled option" },
};
