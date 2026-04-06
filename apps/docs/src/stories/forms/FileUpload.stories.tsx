import type { Meta, StoryObj } from "@storybook/react";
import { FileUpload } from "@7d/ui";
import { useState } from "react";

const meta: Meta<typeof FileUpload> = {
  title: "Forms/FileUpload",
  component: FileUpload,
  tags: ["autodocs"],
};

export default meta;
type Story = StoryObj<typeof FileUpload>;

export const Default: Story = {
  render: () => {
    const [files, setFiles] = useState<File[]>([]);
    return (
      <div className="w-96">
        <FileUpload
          files={files}
          onFilesChange={setFiles}
          label="Upload documents"
          hint="PDF, DOC, or DOCX up to 10MB"
          accept=".pdf,.doc,.docx"
          maxSizeBytes={10 * 1024 * 1024}
        />
      </div>
    );
  },
};

export const Multiple: Story = {
  render: () => {
    const [files, setFiles] = useState<File[]>([]);
    return (
      <div className="w-96">
        <FileUpload
          files={files}
          onFilesChange={setFiles}
          label="Upload images"
          accept="image/*"
          multiple
        />
      </div>
    );
  },
};

export const WithError: Story = {
  render: () => (
    <div className="w-96">
      <FileUpload
        files={[]}
        onFilesChange={() => {}}
        label="Upload required file"
        error="Please upload a file to continue."
      />
    </div>
  ),
};
