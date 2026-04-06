import type { Meta, StoryObj } from "@storybook/react";
import { Input } from "@7d/ui";

const meta: Meta<typeof Input> = {
  title: "Primitives/Input",
  component: Input,
  tags: ["autodocs"],
  args: {
    placeholder: "Enter value…",
  },
};

export default meta;
type Story = StoryObj<typeof Input>;

export const Default: Story = {};

export const Sizes: Story = {
  render: () => (
    <div className="flex flex-col gap-3 w-64">
      <Input size="sm" placeholder="Small" />
      <Input size="md" placeholder="Medium" />
      <Input size="lg" placeholder="Large" />
    </div>
  ),
};

export const WithError: Story = {
  args: {
    error: true,
    placeholder: "Invalid value",
  },
};

export const Disabled: Story = {
  args: { disabled: true, value: "Read-only value" },
};
