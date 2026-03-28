import prisma from '../lib/prisma';
import { KYCService } from './kyc.service';
import { KYCStatus, KycCustomer } from '@prisma/client';

jest.mock('../lib/prisma', () => ({
  __esModule: true,
  default: {
    user: {
      findUnique: jest.fn(),
    },
    kycCustomer: {
      findUnique: jest.fn(),
      upsert: jest.fn(),
      update: jest.fn(),
    },
  },
}));

describe('KYCService', () => {
  let kycService: KYCService;
  const mockPublicKey = 'GABC...123';
  const mockUserId = 'user-123';

  beforeEach(() => {
    kycService = new KYCService();
    jest.clearAllMocks();
  });

  describe('getKycStatus', () => {
    it('should return null if user or kycCustomer is not found', async () => {
      (prisma.user.findUnique as jest.Mock).mockResolvedValue(null);
      const status = await kycService.getKycStatus(mockPublicKey);
      expect(status).toBeNull();
    });

    it('should return KYC record if found', async () => {
      const mockKyc = { userId: mockUserId, status: KYCStatus.PENDING } as KycCustomer;
      (prisma.user.findUnique as jest.Mock).mockResolvedValue({
        id: mockUserId,
        kycCustomer: mockKyc,
      });

      const status = await kycService.getKycStatus(mockPublicKey);
      expect(status).toEqual(mockKyc);
    });
  });

  describe('submitKycData', () => {
    it('should update kyc data and set status to PENDING', async () => {
      const mockUser = { id: mockUserId, publicKey: mockPublicKey };
      const mockData = { firstName: 'Alice', lastName: 'Liddell' };
      const mockResult = { ...mockData, userId: mockUserId, status: KYCStatus.PENDING } as KycCustomer;

      (prisma.user.findUnique as jest.Mock).mockResolvedValue(mockUser);
      (prisma.kycCustomer.upsert as jest.Mock).mockResolvedValue(mockResult);

      const result = await kycService.submitKycData(mockPublicKey, mockData);

      expect(prisma.user.findUnique).toHaveBeenCalledWith({
        where: { publicKey: mockPublicKey },
      });
      expect(prisma.kycCustomer.upsert).toHaveBeenCalledWith({
        where: { userId: mockUserId },
        update: { ...mockData, status: KYCStatus.PENDING },
        create: { ...mockData, userId: mockUserId, status: KYCStatus.PENDING },
      });
      expect(result).toEqual(mockResult);
    });

    it('should throw if user is not found', async () => {
      (prisma.user.findUnique as jest.Mock).mockResolvedValue(null);
      await expect(kycService.submitKycData(mockPublicKey, {})).rejects.toThrow();
    });
  });

  describe('adminUpdateStatus', () => {
    it('should update status and return KYC record', async () => {
      const mockUser = { id: mockUserId, publicKey: mockPublicKey };
      const mockResult = { userId: mockUserId, status: KYCStatus.ACCEPTED } as KycCustomer;

      (prisma.user.findUnique as jest.Mock).mockResolvedValue(mockUser);
      (prisma.kycCustomer.update as jest.Mock).mockResolvedValue(mockResult);

      const result = await kycService.adminUpdateStatus(mockPublicKey, KYCStatus.ACCEPTED);

      expect(prisma.kycCustomer.update).toHaveBeenCalledWith({
        where: { userId: mockUserId },
        data: { status: KYCStatus.ACCEPTED },
      });
      expect(result).toEqual(mockResult);
    });
  });
});
