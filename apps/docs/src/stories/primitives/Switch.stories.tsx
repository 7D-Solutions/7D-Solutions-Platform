import type { Meta, StoryObj } from "@storybook/react";
import { Switch } from "@7d/ui";
import { useState } from "react";

const meta: Meta<typeof Switch> = {
  title: "Primitives/Switch",
  component: Switch,
  tags: ["autodocs"],
};

export default meta;
type Story = StoryObj<typeof Switch>;

export const Default: Story = {
  render: () => {
    const [on, setOn] = useState(false);
    return <Switch checked={on} onChange={setOn} label="Enable notifications" />;
  },
};

export const On: Story = {
  render: () => <Switch checked onChange={() => {}} label="Feature enabled" />,
};

export const Disabled: Story = {
  render: () => <Switch checked={false} onChange={() => {}} label="Disabled" disabled />,
};
