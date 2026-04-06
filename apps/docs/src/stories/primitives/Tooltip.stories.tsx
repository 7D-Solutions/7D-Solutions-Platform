import type { Meta, StoryObj } from "@storybook/react";
import { Tooltip, Button } from "@7d/ui";

const meta: Meta<typeof Tooltip> = {
  title: "Primitives/Tooltip",
  component: Tooltip,
  tags: ["autodocs"],
  parameters: { layout: "centered" },
};

export default meta;
type Story = StoryObj<typeof Tooltip>;

export const Default: Story = {
  render: () => (
    <Tooltip content="This is a tooltip" delay={0}>
      <Button variant="outline">Hover me</Button>
    </Tooltip>
  ),
};

export const Placements: Story = {
  render: () => (
    <div className="grid grid-cols-2 gap-8 p-16">
      {(["top", "bottom", "left", "right"] as const).map((p) => (
        <Tooltip key={p} content={`Tooltip ${p}`} placement={p} delay={0}>
          <Button variant="outline" size="sm">{p}</Button>
        </Tooltip>
      ))}
    </div>
  ),
};
