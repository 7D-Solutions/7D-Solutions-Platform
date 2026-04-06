import type { Meta, StoryObj } from "@storybook/react";
import { RadioGroup } from "@7d/ui";

const OPTIONS = [
  { value: "apple", label: "Apple" },
  { value: "banana", label: "Banana" },
  { value: "cherry", label: "Cherry" },
];

const meta: Meta<typeof RadioGroup> = {
  title: "Primitives/RadioGroup",
  component: RadioGroup,
  tags: ["autodocs"],
  args: {
    name: "fruit",
    options: OPTIONS,
    legend: "Pick a fruit",
  },
};

export default meta;
type Story = StoryObj<typeof RadioGroup>;

export const Vertical: Story = {};

export const Horizontal: Story = {
  args: { orientation: "horizontal" },
};

export const WithError: Story = {
  args: { error: true },
};

export const Disabled: Story = {
  args: { disabled: true, value: "banana" },
};
