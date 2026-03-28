import { KYCStatus, KycCustomer } from '@prisma/client';
import prisma from '../lib/prisma';

/**
 * KYCService handles SEP-12 (KYC) operations.
 */
export class KYCService {
  /**
   * Retrieves the KYC status for a given user public key.
   * 
   * @param publicKey - The public key of the user.
   * @returns The KYC record or null if not found.
   */
  public async getKycStatus(publicKey: string): Promise<KycCustomer | null> {
    const user = await prisma.user.findUnique({
      where: { publicKey },
      include: { kycCustomer: true },
    });

    return user?.kycCustomer || null;
  }

  /**
   * Submits or updates KYC data for a user.
   * Sets the status to PENDING upon submission.
   * 
   * @param publicKey - The public key of the user.
   * @param data - The KYC fields (firstName, lastName, email, etc.).
   * @returns The updated KYC record.
   */
  public async submitKycData(
    publicKey: string,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    data: { firstName?: string; lastName?: string; email?: string; extraFields?: any }
  ): Promise<KycCustomer> {
    const user = await prisma.user.findUnique({
      where: { publicKey },
    });

    if (!user) {
      throw new Error(`User with public key ${publicKey} not found`);
    }

    return await prisma.kycCustomer.upsert({
      where: { userId: user.id },
      update: {
        ...data,
        status: KYCStatus.PENDING,
      },
      create: {
        ...data,
        userId: user.id,
        status: KYCStatus.PENDING,
      },
    });
  }

  /**
   * Updates the KYC status for a user (Admin utility).
   * 
   * @param publicKey - The public key of the user.
   * @param status - The new KYC status (ACCEPTED, REJECTED).
   * @returns The updated KYC record.
   */
  public async adminUpdateStatus(
    publicKey: string,
    status: KYCStatus
  ): Promise<KycCustomer> {
    const user = await prisma.user.findUnique({
      where: { publicKey },
    });

    if (!user) {
      throw new Error(`User with public key ${publicKey} not found`);
    }

    return await prisma.kycCustomer.update({
      where: { userId: user.id },
      data: { status },
    });
  }
}
