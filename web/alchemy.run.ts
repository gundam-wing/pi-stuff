// alchemy.run.ts
import alchemy from "alchemy";
import {
    Bucket,
    BucketLifecycleRule,
    Queue,
    QueuePolicy,
    Function,
    FunctionUrl,
    Role,
    Policy,
} from "alchemy/aws";

// Initialize the Alchemy app
const app = await alchemy("camera-stream");

// =============================================================================
// S3 Bucket for HLS segments
// =============================================================================
export const streamBucket = await Bucket("camera-stream-bucket", {
    bucketName: `camera-stream-${app.deploymentId}`,

    // Enable CORS for web player access
    corsConfiguration: {
        corsRules: [{
            allowedHeaders: ["*"],
            allowedMethods: ["GET", "HEAD"],
            allowedOrigins: ["*"], // Restrict this to your domain in production
            maxAgeSeconds: 3000,
        }],
    },

    // Lifecycle rule to auto-delete old segments
    lifecycleRules: [{
        id: "delete-old-segments",
        enabled: true,
        prefix: "live/",
        expiration: {
            days: 1, // Delete segments older than 1 day
        },
    }] as BucketLifecycleRule[],

    // Public read access for video segments
    publicAccessBlock: {
        blockPublicAcls: false,
        blockPublicPolicy: false,
        ignorePublicAcls: false,
        restrictPublicBuckets: false,
    },
});

// Make bucket contents publicly readable
await Policy("stream-bucket-policy", {
    policyDocument: {
        Version: "2012-10-17",
        Statement: [{
            Sid: "PublicReadGetObject",
            Effect: "Allow",
            Principal: "*",
            Action: "s3:GetObject",
            Resource: `${streamBucket.arn}/live/*`,
        }],
    },
    bucket: streamBucket.bucketName,
});

// =============================================================================
// SQS Queue for Pi commands
// =============================================================================
export const commandQueue = await Queue("camera-commands", {
    queueName: "camera-commands",
    visibilityTimeout: 60, // Must be >= Lambda timeout
    messageRetentionPeriod: 3600, // 1 hour
});

// Grant queue access policy
await QueuePolicy("command-queue-policy", {
    queueUrl: commandQueue.url,
    policy: {
        Version: "2012-10-17",
        Statement: [{
            Effect: "Allow",
            Principal: {
                AWS: "*", // Restrict to your account ARN in production
            },
            Action: [
                "sqs:SendMessage",
                "sqs:ReceiveMessage",
                "sqs:DeleteMessage",
            ],
            Resource: commandQueue.arn,
        }],
    },
});

// =============================================================================
// IAM Role for Lambda functions
// =============================================================================
const lambdaRole = await Role("lambda-execution-role", {
    roleName: "camera-lambda-role",
    assumeRolePolicy: {
        Version: "2012-10-17",
        Statement: [{
            Effect: "Allow",
            Principal: {
                Service: "lambda.amazonaws.com",
            },
            Action: "sts:AssumeRole",
        }],
    },
    managedPolicyArns: [
        "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole",
    ],
    inlinePolicies: [{
        policyName: "camera-permissions",
        policy: {
            Version: "2012-10-17",
            Statement: [
                {
                    Effect: "Allow",
                    Action: [
                        "sqs:SendMessage",
                        "sqs:GetQueueUrl",
                    ],
                    Resource: commandQueue.arn,
                },
            ],
        },
    }],
});

// =============================================================================
// Lambda Functions for API
// =============================================================================

// Start streaming function
export const startStreamFn = await Function("start-stream", {
    functionName: "camera-start-stream",
    runtime: "nodejs20.x",
    handler: "index.handler",
    role: lambdaRole.arn,
    timeout: 10,
    environment: {
        variables: {
            QUEUE_URL: commandQueue.url,
        },
    },
    code: {
        zipFile: `
const { SQSClient, SendMessageCommand } = require("@aws-sdk/client-sqs");
const sqs = new SQSClient();

exports.handler = async (event) => {
  try {
    await sqs.send(new SendMessageCommand({
      QueueUrl: process.env.QUEUE_URL,
      MessageBody: JSON.stringify({ command: "START" }),
    }));
    
    return {
      statusCode: 200,
      headers: {
        'Access-Control-Allow-Origin': '*',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ message: "Stream starting..." }),
    };
  } catch (error) {
    console.error(error);
    return {
      statusCode: 500,
      body: JSON.stringify({ error: error.message }),
    };
  }
};
    `,
    },
});

// Stop streaming function
export const stopStreamFn = await Function("stop-stream", {
    functionName: "camera-stop-stream",
    runtime: "nodejs20.x",
    handler: "index.handler",
    role: lambdaRole.arn,
    timeout: 10,
    environment: {
        variables: {
            QUEUE_URL: commandQueue.url,
        },
    },
    code: {
        zipFile: `
const { SQSClient, SendMessageCommand } = require("@aws-sdk/client-sqs");
const sqs = new SQSClient();

exports.handler = async (event) => {
  try {
    await sqs.send(new SendMessageCommand({
      QueueUrl: process.env.QUEUE_URL,
      MessageBody: JSON.stringify({ command: "STOP" }),
    }));
    
    return {
      statusCode: 200,
      headers: {
        'Access-Control-Allow-Origin': '*',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ message: "Stream stopping..." }),
    };
  } catch (error) {
    console.error(error);
    return {
      statusCode: 500,
      body: JSON.stringify({ error: error.message }),
    };
  }
};
    `,
    },
});

// Status check function
export const statusFn = await Function("stream-status", {
    functionName: "camera-stream-status",
    runtime: "nodejs20.x",
    handler: "index.handler",
    role: lambdaRole.arn,
    timeout: 10,
    environment: {
        variables: {
            BUCKET_NAME: streamBucket.bucketName,
        },
    },
    code: {
        zipFile: `
const { S3Client, HeadObjectCommand } = require("@aws-sdk/client-s3");
const s3 = new S3Client();

exports.handler = async (event) => {
  try {
    // Check if playlist exists and is recent
    const result = await s3.send(new HeadObjectCommand({
      Bucket: process.env.BUCKET_NAME,
      Key: "live/stream.m3u8",
    }));
    
    const lastModified = new Date(result.LastModified);
    const now = new Date();
    const ageSeconds = (now - lastModified) / 1000;
    
    return {
      statusCode: 200,
      headers: {
        'Access-Control-Allow-Origin': '*',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        streaming: ageSeconds < 30, // Active if updated in last 30 seconds
        lastUpdate: lastModified.toISOString(),
      }),
    };
  } catch (error) {
    // Playlist doesn't exist - not streaming
    return {
      statusCode: 200,
      headers: {
        'Access-Control-Allow-Origin': '*',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        streaming: false,
      }),
    };
  }
};
    `,
    },
});

// =============================================================================
// Lambda Function URLs (for simple HTTP access)
// =============================================================================
export const startUrl = await FunctionUrl("start-url", {
    functionName: startStreamFn.functionName,
    authorizationType: "NONE", // Use AWS_IAM in production
    cors: {
        allowOrigins: ["*"],
        allowMethods: ["POST", "GET"],
        allowHeaders: ["*"],
        maxAge: 300,
    },
});

export const stopUrl = await FunctionUrl("stop-url", {
    functionName: stopStreamFn.functionName,
    authorizationType: "NONE",
    cors: {
        allowOrigins: ["*"],
        allowMethods: ["POST", "GET"],
        allowHeaders: ["*"],
        maxAge: 300,
    },
});

export const statusUrl = await FunctionUrl("status-url", {
    functionName: statusFn.functionName,
    authorizationType: "NONE",
    cors: {
        allowOrigins: ["*"],
        allowMethods: ["GET"],
        allowHeaders: ["*"],
        maxAge: 300,
    },
});

// =============================================================================
// Output important values
// =============================================================================
console.log("\n=== Camera Stream Infrastructure ===");
console.log(`S3 Bucket: ${streamBucket.bucketName}`);
console.log(`Stream URL: https://${streamBucket.bucketName}.s3.amazonaws.com/live/stream.m3u8`);
console.log(`\nSQS Queue URL: ${commandQueue.url}`);
console.log(`\nAPI Endpoints:`);
console.log(`  Start:  ${startUrl.functionUrl}`);
console.log(`  Stop:   ${stopUrl.functionUrl}`);
console.log(`  Status: ${statusUrl.functionUrl}`);
console.log("\n=== Configuration for Pi ===");
console.log(`QUEUE_URL=${commandQueue.url}`);
console.log(`S3_BUCKET=${streamBucket.bucketName}`);
console.log(`S3_PREFIX=live`);

// Finalize - cleanup any orphaned resources
await app.finalize();